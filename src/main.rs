use spider::website::Website;
use spider::page::Page;
use spider::configuration::{
    WaitForIdleNetwork, WaitForSelector, CaptureScreenshotFormat, CaptureScreenshotParams,
    ClipViewport, ScreenShotConfig, ScreenshotParams,
};

use std::time::Duration;

#[spider::tokio::main] // uses spider's re-export of tokio
async fn main() {
    // Assume you already parsed args: args.url: String,
    // args.headless_idle_ms: u64, args.selector: Option<String>, args.screenshot: bool

    let mut site = Website::new(&args.url);

    // --- chrome & smart fallback related toggles ---
    // (Requires features: "smart", "chrome"; chrome only takes effect if a Chrome is available.)
    site
        // Auto-wait for network idleness in headless mode (wrap Duration in WaitForIdleNetwork)
        .with_wait_for_idle_network(Some(WaitForIdleNetwork::new(
            Some(Duration::from_millis(args.headless_idle_ms)),
        )))
        // Optionally wait for a selector if you want deterministic “rendered” state:
        .with_wait_for_selector(args.selector.as_ref().map(|sel| {
            WaitForSelector::new(Some(Duration::from_secs(30)), sel.to_string())
        }))
        // Optional: capture screenshot bytes later via Page::screenshot; if you prefer
        // the crate to save screenshots automatically for *every* page, configure:
        .with_screenshot(
            if args.screenshot {
                Some(ScreenShotConfig::new(
                    ScreenshotParams::new(
                        CaptureScreenshotParams {
                            format: Some(CaptureScreenshotFormat::Png),
                            quality: None,
                            clip: None,
                            from_surface: None,
                            capture_beyond_viewport: Some(true),
                        },
                        Some(true),  // full_page
                        Some(false), // omit_background
                    ),
                    true,  // also keep bytes on Page (when supported)
                    false, // don't auto-save to disk
                    None,  // no output dir
                ))
            } else {
                None
            }
        )
        // A few practical toggles when doing dynamic pages / bot-avoidance:
        .with_modify_headers(true)   // make headers look more browser-like
        .with_stealth(true)          // stealth anti-bot heuristics (needs "chrome")
        .with_block_assets(true)     // focus on HTML (you can turn off if you need CSS/JS files)
        .with_caching(true);         // honor HTTP cache (needs "cache" or "chrome")

    // If you have a remote Chrome, point to it; otherwise spider will manage a local Chrome.
    // std::env::set_var("CHROME_URL", "ws://localhost:9222/devtools/browser/<id>");

    // Smart scraping: HTTP first; render with Chrome only when needed.
    site.scrape_smart().await; // requires "smart" feature. :contentReference[oaicite:5]{index=5}

    // --- Choose the page we care about ---
    // pages_opt: Option<&Vec<Page>>
    let pages_opt = site.get_pages();

    // Prefer the page that matches the original or final URL; else fall back to first page.
    let chosen: &Page = pages_opt
        .and_then(|pages| {
            pages.iter().find(|p| {
                p.get_url() == args.url || p.get_url_final() == args.url
            })
        })
        .or_else(|| pages_opt.and_then(|pages| pages.first()))
        .expect("no page captured"); // handle the 'no page' case however you like

    // --- Collect the output you wanted ---
    // HTML
    let html = chosen.get_html(); // or String::from_utf8_lossy(chosen.get_html_bytes_u8())

    // Redirect tracking
    // Note: for pure Chrome-rendered fetches the crate notes that the
    // final redirect destination is *not implemented in chrome mode*.
    // You still get the original URL via get_url()/get_url_final().
    let url_original = chosen.get_url().to_string();
    let url_final = chosen.get_url_final().to_string(); // falls back to original if None

    // WAF / bot tracking (Cloudflare/others are reported via anti_bot_tech, with a boolean waf_check)
    let waf = chosen.waf_check;
    let anti_bot = format!("{:?}", chosen.anti_bot_tech);

    // Optional screenshot (requires features "chrome" + "chrome_store_page")
    let screenshot_path = if args.screenshot {
        use std::path::PathBuf;
        let path = PathBuf::from("screenshot.png");
        // Full-page PNG, omit_background = false, no custom clip
        let _bytes = chosen
            .screenshot(
                true,
                false,
                CaptureScreenshotFormat::Png,
                None,
                Some(&path),
                None::<ClipViewport>,
            )
            .await;
        Some(path.to_string_lossy().to_string())
    } else {
        None
    };

    // Your prototype output
    println!(
        "{{\"url_original\":\"{}\",\"url_final\":\"{}\",\"redirected\":{},\"waf_check\":{},\"anti_bot\":\"{}\",\"screenshot_path\":{},\"html_len\":{}}}",
        url_original,
        url_final,
        (url_final != url_original),
        waf,
        anti_bot,
        screenshot_path
            .as_ref()
            .map(|s| format!("\"{}\"", s))
            .unwrap_or_else(|| "null".to_string()),
        html.len()
    );

    // If you want to actually return the HTML:
    // println!("{}", html);
}

