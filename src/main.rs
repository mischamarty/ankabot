use anyhow::Result;
use clap::Parser;
use serde::Serialize;
use spider::configuration::WaitForIdleNetwork;
use spider::features::chrome_common::CaptureScreenshotFormat;
use spider::page::AntiBotTech;
use spider::page::Page;
use spider::website::{self, Website};

use std::time::{Duration, Instant};
use ua_generator::ua::spoof_ua;

/// Simple probe utility using spider's smart HTTP→headless fallback.
#[derive(Parser, Debug)]
#[command(
    name = "spider-probe",
    about = "HTTP-first fetch with headless fallback"
)]
struct Args {
    /// URL to fetch
    url: String,

    /// Optional path to save a screenshot (PNG). Example: ./page.png
    #[arg(long)]
    screenshot: Option<String>,

    /// Milliseconds to wait for network idle when in headless mode
    #[arg(long, default_value = "1500")]
    headless_idle_ms: u64,

    /// Request timeout (seconds)
    #[arg(long, default_value = "30")]
    timeout_s: u64,
}

#[derive(Serialize)]
struct ProbeResult {
    input_url: String,
    final_url: String,
    http_status: u16,
    redirected: bool,
    requires_javascript: bool,
    waf_detected: bool,
    anti_bot_vendor: Option<String>,
    js_challenge_page: bool,
    screenshot_path: Option<String>,
    html: String,
    /// Total time spent in milliseconds
    elapsed_ms: u64,
    /// Number of pages captured during the crawl
    pages_crawled: usize,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let start = Instant::now();

    // Build a Website rooted at the provided URL but limit to just the root (depth=0).
    let mut site = Website::new(&args.url);
    let ua = spoof_ua();
    site.with_depth(0) // single page
        .with_limit(1) // ensure we crawl exactly one page
        .with_user_agent(Some(ua)) // pretend to be a real browser
        .with_respect_robots_txt(true) // be polite by default
        .with_request_timeout(Some(Duration::from_secs(args.timeout_s)))
        .with_redirect_limit(10)
        // Wait a bit for the page to settle when using headless
        .with_wait_for_idle_network(Some(WaitForIdleNetwork::new(Some(Duration::from_millis(
            args.headless_idle_ms,
        )))));

    // Smart scrape = HTTP first; escalate to Chrome only if needed (requires `smart` + `chrome` features).
    site.scrape_smart().await;

    let pages = match site.get_pages() {
        Some(p) if !p.is_empty() => p,
        _ => {
            eprintln!(
                "debug: status={:?}, initial_status={:?}, requires_js={}, links_found={}",
                site.get_status(),
                site.get_initial_status_code(),
                site.get_requires_javascript(),
                site.get_links().len()
            );

            // Produce a minimal result rather than failing.
            let http_status = site.get_initial_status_code().as_u16();
            let elapsed_ms = start.elapsed().as_millis() as u64;
            let out = ProbeResult {
                input_url: args.url.clone(),
                final_url: args.url.clone(),
                http_status,
                redirected: false,
                requires_javascript: site.get_requires_javascript(),
                waf_detected: false,
                anti_bot_vendor: None,
                js_challenge_page: false,
                screenshot_path: None,
                html: String::new(),
                elapsed_ms,
                pages_crawled: 0,
            };
            println!("{}", serde_json::to_string_pretty(&out)?);
            return Ok(());
        }
    };
    let pages_crawled = pages.len();

    // Find the page corresponding to the root URL (there should be exactly one at depth=0).
    let mut chosen: Option<&Page> = None;
    for p in pages.iter() {
        // Prefer exact match on final URL when available, otherwise the original.
        if p.get_url() == args.url || p.get_url_final() == args.url {
            chosen = Some(p);
            break;
        }
    }
    // Fallback if the above heuristic didn't match
    let page = chosen.unwrap_or_else(|| pages.first().expect("no page captured"));

    let status = page.status_code.as_u16();
    let (input_url, final_url) = (page.get_url().to_string(), page.get_url_final().to_string());
    // Note: final_redirect_destination is not populated when Chrome handled the fetch path.
    // In that case redirected==false may simply mean "unknown from HTTP path".
    let redirected = final_url != input_url;

    // Detect WAF / anti-bot
    let waf_detected = page.waf_check;
    let anti_bot_vendor = match page.anti_bot_tech {
        AntiBotTech::None => None,
        ref other => Some(format!("{:?}", other)),
    };
    let js_challenge_page = website::is_safe_javascript_challenge(page);

    // Optionally capture a screenshot (works when Chrome path was used; requires `chrome_store_page`).
    let screenshot_path = if let Some(path) = &args.screenshot {
        // full_page=true, omit_bg=true, format=PNG, quality=None, output_path=Some(path), clip=None
        let _bytes = page
            .screenshot(
                true,
                true,
                CaptureScreenshotFormat::Png,
                None,
                Some(path),
                None,
            )
            .await;
        Some(path.clone())
    } else {
        None
    };

    let html = String::from_utf8_lossy(page.get_html_bytes_u8()).to_string();
    let elapsed_ms = start.elapsed().as_millis() as u64;

    let out = ProbeResult {
        input_url,
        final_url,
        http_status: status,
        redirected,
        requires_javascript: site.get_requires_javascript(),
        waf_detected,
        anti_bot_vendor,
        js_challenge_page,
        screenshot_path,
        html,
        elapsed_ms,
        pages_crawled,
    };

    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}
