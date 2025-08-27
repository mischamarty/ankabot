use anyhow::{Context, Result};
use clap::Parser;
use serde::Serialize;
use std::{path::PathBuf, time::Duration};

fn default_pdf_path(url: &str) -> PathBuf {
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let host = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_else(|| "page".to_string());
    PathBuf::from(format!("{}-{}.pdf", host, ts))
}

#[derive(Parser, Debug)]
struct Cli {
    /// URL to fetch
    url: String,
    /// Save a PDF of the page (defaults to <host>-<timestamp>.pdf if not provided)
    #[arg(long, conflicts_with = "no_pdf")]
    pdf: Option<PathBuf>,
    /// Do not save a PDF (overrides default behavior)
    #[arg(long)]
    no_pdf: bool,
    /// Always render with headless Chrome
    #[arg(long)]
    force_chrome: bool,
    /// Milliseconds to allow page JS to settle under Chrome
    #[arg(long, default_value_t = 6000)]
    render_ms: u64,
    /// Save a screenshot (PNG) when Chrome is used
    #[arg(long)]
    screenshot: Option<PathBuf>,
}

#[derive(Serialize)]
struct Output {
    input_url: String,
    final_url: String,
    http_status: u16,
    redirected: bool,
    requires_javascript: bool,
    waf_detected: bool,
    anti_bot_vendor: Option<String>,
    js_challenge_page: bool,
    screenshot_path: Option<String>,
    pdf_path: Option<String>,
    html: String,
    elapsed_ms: u64,
    pages_crawled: u32,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Cli::parse();

    let want_pdf = !args.no_pdf || args.pdf.is_some();

    if !args.force_chrome && !want_pdf {
        if let Ok(http_res) = fetch_http(&args.url).await {
            let looks_empty = http_res.html.trim().is_empty()
                || http_res.html.len() < 512
                || !http_res.html.to_lowercase().contains("<body");
            let needs_js = looks_empty || http_res.links_found == 0;

            if !needs_js {
                print_json(Output {
                    input_url: args.url,
                    final_url: http_res.final_url,
                    http_status: http_res.status,
                    redirected: http_res.redirected,
                    requires_javascript: false,
                    waf_detected: http_res.waf_detected,
                    anti_bot_vendor: http_res.anti_bot_vendor,
                    js_challenge_page: false,
                    screenshot_path: None,
                    pdf_path: None,
                    html: http_res.html,
                    elapsed_ms: http_res.elapsed_ms,
                    pages_crawled: 0,
                })?;
                return Ok(());
            }
        }
    }

    let pdf_path = if want_pdf {
        Some(
            args.pdf
                .clone()
                .unwrap_or_else(|| default_pdf_path(&args.url)),
        )
    } else {
        None
    };

    let chrome = fetch_with_chrome(
        &args.url,
        Duration::from_millis(args.render_ms),
        args.screenshot.as_ref(),
        pdf_path.as_ref(),
    )
    .context("headless-chrome render failed")?;

    print_json(Output {
        input_url: args.url,
        final_url: chrome.final_url,
        http_status: chrome.status.unwrap_or(200),
        redirected: chrome.redirected,
        requires_javascript: true,
        waf_detected: chrome.waf_detected,
        anti_bot_vendor: chrome.anti_bot_vendor,
        js_challenge_page: chrome.js_challenge,
        screenshot_path: chrome.screenshot_path,
        pdf_path: chrome.pdf_path,
        html: chrome.html,
        elapsed_ms: chrome.elapsed_ms,
        pages_crawled: 1,
    })?;

    Ok(())
}

struct HttpRes {
    final_url: String,
    status: u16,
    redirected: bool,
    html: String,
    links_found: usize,
    elapsed_ms: u64,
    waf_detected: bool,
    anti_bot_vendor: Option<String>,
}

async fn fetch_http(url: &str) -> Result<HttpRes> {
    let client = reqwest::Client::builder()
        .user_agent(ua_generator::ua::spoof_ua())
        .redirect(reqwest::redirect::Policy::limited(8))
        .gzip(true)
        .brotli(true)
        .deflate(true)
        .timeout(Duration::from_secs(10))
        .build()?;

    let start = std::time::Instant::now();
    let resp = client.get(url).send().await?;
    let status = resp.status().as_u16();
    let final_url = resp.url().to_string();
    let redirected = final_url != url;

    let is_html = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.to_ascii_lowercase().contains("text/html"))
        .unwrap_or(true);

    let html = if is_html {
        resp.text().await?
    } else {
        String::new()
    };

    let links_found = html.matches("<a ").count();
    let elapsed_ms = start.elapsed().as_millis() as u64;

    Ok(HttpRes {
        final_url,
        status,
        redirected,
        html,
        links_found,
        elapsed_ms,
        waf_detected: false,
        anti_bot_vendor: None,
    })
}

struct ChromeRes {
    final_url: String,
    status: Option<u16>,
    redirected: bool,
    html: String,
    elapsed_ms: u64,
    screenshot_path: Option<String>,
    pdf_path: Option<String>,
    waf_detected: bool,
    anti_bot_vendor: Option<String>,
    js_challenge: bool,
}

fn fetch_with_chrome(
    url: &str,
    budget: Duration,
    screenshot: Option<&PathBuf>,
    pdf_path: Option<&PathBuf>,
) -> Result<ChromeRes> {
    use headless_chrome::{
        protocol::cdp::Page::CaptureScreenshotFormatOption, types::PrintToPdfOptions, Browser,
        LaunchOptionsBuilder,
    };
    use std::ffi::OsStr;

    let launch_opts = LaunchOptionsBuilder::default()
        .headless(true)
        .args(vec![
            OsStr::new("--headless=new"),
            OsStr::new("--disable-gpu"),
            OsStr::new("--disable-dev-shm-usage"),
            OsStr::new("--no-first-run"),
            OsStr::new("--no-default-browser-check"),
            OsStr::new("--hide-scrollbars"),
        ])
        .build()
        .unwrap();

    let browser = Browser::new(launch_opts)?;
    let tab = browser.new_tab()?;

    let start = std::time::Instant::now();
    tab.set_user_agent(&ua_generator::ua::spoof_ua(), None, None)?;
    tab.navigate_to(url)?;
    tab.wait_until_navigated()?;
    std::thread::sleep(budget);

    let body_text = tab
        .evaluate(
            "document.body ? document.body.innerText.slice(0, 4096) : ''",
            false,
        )?
        .value
        .map(|v| v.to_string());
    let challenge = body_text
        .as_deref()
        .map(|t| {
            let l = t.to_ascii_lowercase();
            l.contains("checking your browser")
                || l.contains("verifying you are human")
                || l.contains("press and hold")
        })
        .unwrap_or(false);

    let html = tab.get_content()?;
    let final_url = tab.get_url();
    let redirected = final_url != url;

    let screenshot_path = if let Some(p) = screenshot {
        let png = tab.capture_screenshot(CaptureScreenshotFormatOption::Png, None, None, true)?;
        std::fs::write(p, png)?;
        Some(p.display().to_string())
    } else {
        None
    };

    let mut pdf_saved: Option<String> = None;
    if let Some(p) = pdf_path {
        let bytes = tab.print_to_pdf(Some(PrintToPdfOptions {
            print_background: Some(true),
            prefer_css_page_size: Some(true),
            margin_top: Some(0.0),
            margin_bottom: Some(0.0),
            margin_left: Some(0.0),
            margin_right: Some(0.0),
            ..Default::default()
        }))?;
        std::fs::write(p, &bytes)?;
        pdf_saved = Some(p.display().to_string());
    }

    Ok(ChromeRes {
        final_url,
        status: None,
        redirected,
        html,
        elapsed_ms: start.elapsed().as_millis() as u64,
        screenshot_path,
        pdf_path: pdf_saved,
        waf_detected: challenge,
        anti_bot_vendor: None,
        js_challenge: challenge,
    })
}

fn print_json<T: Serialize>(v: T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&v)?);
    Ok(())
}
