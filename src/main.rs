use anyhow::{anyhow, Context, Result};
use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

struct RunPaths {
    run_dir: PathBuf,
    pdf: PathBuf,
    png: PathBuf,
    dom_html: PathBuf,
    http_raw: PathBuf,
    console_log: PathBuf,
    network_log: PathBuf,
    result_json: PathBuf,
}

fn new_run_paths(
    out_root: Option<PathBuf>,
    run_dir_override: Option<PathBuf>,
    url: &str,
) -> anyhow::Result<RunPaths> {
    let root = out_root.unwrap_or_else(|| PathBuf::from("out"));
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let host = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_else(|| "page".into());
    let run = run_dir_override.unwrap_or_else(|| root.join(format!("{}-{}", host, ts)));
    std::fs::create_dir_all(&run)?;
    let abs = dunce::canonicalize(&run).unwrap_or(run.clone());
    Ok(RunPaths {
        run_dir: abs.clone(),
        pdf: abs.join("page.pdf"),
        png: abs.join("snap.png"),
        dom_html: abs.join("dom.html"),
        http_raw: abs.join("http_raw.html"),
        console_log: abs.join("console.log"),
        network_log: abs.join("network.txt"),
        result_json: abs.join("result.json"),
    })
}

fn profile_dir(profile: &str, override_dir: Option<PathBuf>) -> PathBuf {
    if let Some(p) = override_dir {
        return p;
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ankabot")
        .join("profiles")
        .join(profile)
}

#[derive(Parser, Debug, Clone)]
struct Cli {
    /// URL to fetch
    url: String,
    /// Legacy no-op alias for compatibility
    #[arg(long, hide = true)]
    pdf: Option<PathBuf>,
    /// Always render with headless Chrome
    #[arg(long)]
    force_chrome: bool,
    /// Output root directory
    #[arg(long, default_value = "./out")]
    out_root: PathBuf,
    /// Override run directory
    #[arg(long)]
    run_dir: Option<PathBuf>,
    /// Overall deadline for page load waits
    #[arg(long, default_value_t = 12000)]
    max_wait_ms: u64,
    /// document.readyState to await ("complete" | "interactive" | "none")
    #[arg(long, default_value = "complete")]
    wait_ready: String,
    /// How long network must stay idle (pending requests 0)
    #[arg(long, default_value_t = 1000)]
    network_idle_ms: u64,
    /// Pending request threshold to still consider the page idle
    #[arg(long, default_value_t = 0)]
    idle_threshold: u64,
    /// Regex of URLs to ignore when calculating network idle
    #[arg(long)]
    idle_ignore: Option<String>,
    /// Minimum DOM text characters for heuristic readiness
    #[arg(long, default_value_t = 1500)]
    heuristic_min_chars: u64,
    /// Optional CSS selector to wait for
    #[arg(long)]
    wait_selector: Option<String>,
    /// Named Chrome profile for persistent sessions
    #[arg(long, default_value = "default")]
    profile: String,
    /// Override the Chrome user-data-dir
    #[arg(long)]
    user_data_dir: Option<PathBuf>,
    /// Import cookies from JSON file
    #[arg(long)]
    import_cookies: Option<PathBuf>,
    /// Export cookies to JSON file
    #[arg(long)]
    export_cookies: Option<PathBuf>,
    /// Locale / Accept-Language override
    #[arg(long)]
    locale: Option<String>,
    /// Timezone override
    #[arg(long)]
    tz: Option<String>,
    /// Geolocation "lat,lon[,accuracy]"
    #[arg(long)]
    geo: Option<String>,
    /// Viewport size "WIDTHxHEIGHT"
    #[arg(long, default_value = "1366x768")]
    window: String,
    /// Device pixel ratio
    #[arg(long, default_value_t = 1.0)]
    dpr: f64,
    /// Emulate a mobile device
    #[arg(long, default_value_t = false)]
    mobile: bool,
    /// Run Chrome in headful mode
    #[arg(long)]
    headful: bool,
    /// Comma-separated list of extension dirs
    #[arg(long)]
    extensions: Option<String>,
    /// Proxy server URL (http:// or socks5://)
    #[arg(long)]
    proxy: Option<String>,
    /// Disable Chrome's virtual time budget
    #[arg(long)]
    no_virtual_time: bool,
    /// Retry in headful mode if headless fails
    #[arg(long)]
    headful_fallback: bool,
    /// Directory for timeout debug artifacts
    #[arg(long, default_value = "out/debug")]
    debug_dir: PathBuf,
    /// Action to take on render timeout
    #[arg(long, value_enum, default_value = "report")]
    on_timeout: OnTimeout,
}

impl Cli {
    fn locale_or_default(&self) -> String {
        self.locale
            .clone()
            .unwrap_or_else(|| "en-US,en;q=0.9".to_string())
    }

    fn window_size(&self) -> (u32, u32) {
        let lower = self.window.to_lowercase();
        let parts: Vec<&str> = lower.split('x').collect();
        if parts.len() == 2 {
            if let (Ok(w), Ok(h)) = (parts[0].parse(), parts[1].parse()) {
                return (w, h);
            }
        }
        (1366, 768)
    }
}

#[derive(Clone, Debug, ValueEnum)]
enum OnTimeout {
    Continue,
    Report,
    Fail,
}

#[derive(Serialize)]
struct TimeoutReport {
    status: &'static str,
    reason: String,
    url: String,
    deadline_ms: u64,
    elapsed_ms: u64,
    wait_branch: String,
    diagnostics: Diagnostics,
    artifacts: Artifacts,
}

#[derive(Serialize)]
struct Diagnostics {
    dom_text_chars: u64,
    images_total: u64,
    images_incomplete: u64,
    pending_requests: u64,
}

#[derive(Serialize)]
struct Artifacts {
    html: String,
    screenshot: String,
    pdf: Option<String>,
}

enum RenderOutcome {
    Success(ChromeRes),
    Timeout(TimeoutReport),
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
    html_path: String,
    elapsed_ms: u64,
    pages_crawled: u32,
    wait_branch: String,
    run_dir: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Cli::parse();
    let run_paths = new_run_paths(Some(args.out_root.clone()), args.run_dir.clone(), &args.url)?;

    if !args.force_chrome {
        if let Ok(http_res) = fetch_http(&args.url, &run_paths.http_raw).await {
            let needs_js = http_res.looks_empty || http_res.links_found == 0;

            if !needs_js {
                let out = Output {
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
                    html_path: run_paths.http_raw.display().to_string(),
                    elapsed_ms: http_res.elapsed_ms,
                    pages_crawled: 0,
                    wait_branch: "ready_state".to_string(),
                    run_dir: run_paths.run_dir.display().to_string(),
                };
                write_json(&run_paths.result_json, &out)?;
                return Ok(());
            }
        }
    }

    let mut chrome_res = render_with_chrome(&args.url, &run_paths, &args);
    if chrome_res.is_err() && args.headful_fallback && !args.headful {
        let mut retry = args.clone();
        retry.headful = true;
        chrome_res = render_with_chrome(&args.url, &run_paths, &retry);
    }
    let outcome = chrome_res.context("headless-chrome render failed")?;

    match outcome {
        RenderOutcome::Success(chrome) => {
            let out = Output {
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
                html_path: chrome.html_path,
                elapsed_ms: chrome.elapsed_ms,
                pages_crawled: 1,
                wait_branch: chrome.wait_branch,
                run_dir: run_paths.run_dir.display().to_string(),
            };
            write_json(&run_paths.result_json, &out)?;
            Ok(())
        }
        RenderOutcome::Timeout(report) => match args.on_timeout {
            OnTimeout::Report => {
                write_json(&run_paths.result_json, &report)?;
                std::process::exit(2);
            }
            OnTimeout::Continue => {
                let TimeoutReport {
                    url,
                    elapsed_ms,
                    wait_branch,
                    artifacts,
                    ..
                } = report;
                let out = Output {
                    input_url: args.url,
                    final_url: url,
                    http_status: 0,
                    redirected: false,
                    requires_javascript: true,
                    waf_detected: false,
                    anti_bot_vendor: None,
                    js_challenge_page: false,
                    screenshot_path: Some(artifacts.screenshot),
                    pdf_path: artifacts.pdf,
                    html_path: artifacts.html,
                    elapsed_ms,
                    pages_crawled: 1,
                    wait_branch,
                    run_dir: run_paths.run_dir.display().to_string(),
                };
                write_json(&run_paths.result_json, &out)?;
                Ok(())
            }
            OnTimeout::Fail => Err(anyhow!(report.reason)),
        },
    }
}

struct HttpRes {
    final_url: String,
    status: u16,
    redirected: bool,
    links_found: usize,
    looks_empty: bool,
    elapsed_ms: u64,
    waf_detected: bool,
    anti_bot_vendor: Option<String>,
}

async fn fetch_http(url: &str, html_path: &Path) -> Result<HttpRes> {
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
    std::fs::write(html_path, &html)?;

    let links_found = html.matches("<a ").count();
    let looks_empty =
        html.trim().is_empty() || html.len() < 512 || !html.to_lowercase().contains("<body");
    let elapsed_ms = start.elapsed().as_millis() as u64;

    Ok(HttpRes {
        final_url,
        status,
        redirected,
        links_found,
        looks_empty,
        elapsed_ms,
        waf_detected: false,
        anti_bot_vendor: None,
    })
}

struct ChromeRes {
    final_url: String,
    status: Option<u16>,
    redirected: bool,
    html_path: String,
    elapsed_ms: u64,
    screenshot_path: Option<String>,
    pdf_path: Option<String>,
    waf_detected: bool,
    anti_bot_vendor: Option<String>,
    js_challenge: bool,
    wait_branch: String,
}

#[derive(Deserialize, Serialize)]
struct CookieJson {
    name: String,
    value: String,
    domain: String,
    path: String,
    secure: bool,
    #[serde(default, rename = "httpOnly")]
    http_only: bool,
    #[serde(default)]
    expires: Option<f64>,
}

fn import_cookies_to_chrome(tab: &headless_chrome::Tab, list: &[CookieJson]) -> Result<()> {
    use headless_chrome::protocol::cdp::Network;

    tab.call_method(Network::Enable {
        max_total_buffer_size: None,
        max_resource_buffer_size: None,
        max_post_data_size: None,
    })?;
    let params: Vec<Network::CookieParam> = list
        .iter()
        .map(|c| Network::CookieParam {
            name: c.name.clone(),
            value: c.value.clone(),
            url: None,
            domain: Some(c.domain.clone()),
            path: Some(c.path.clone()),
            secure: Some(c.secure),
            http_only: Some(c.http_only),
            same_site: None,
            expires: c.expires,
            priority: None,
            same_party: None,
            source_scheme: None,
            source_port: None,
            partition_key: None,
        })
        .collect();
    tab.call_method(Network::SetCookies { cookies: params })?;
    Ok(())
}

fn export_cookies_from_chrome(tab: &headless_chrome::Tab) -> Result<Vec<CookieJson>> {
    use headless_chrome::protocol::cdp::Network;
    tab.call_method(Network::Enable {
        max_total_buffer_size: None,
        max_resource_buffer_size: None,
        max_post_data_size: None,
    })?;
    let all = tab.get_cookies()?;
    let out = all
        .into_iter()
        .map(|c| CookieJson {
            name: c.name,
            value: c.value,
            domain: c.domain,
            path: c.path,
            secure: c.secure,
            http_only: c.http_only,
            expires: Some(c.expires),
        })
        .collect();
    Ok(out)
}

fn build_instrument_js(ignore: &str) -> String {
    format!(
        r#"(() => {{
  if (window.__ankabot) return;
  window.__ankabot = {{ pending: 0 }};
  const IGNORE = new RegExp({:?});
  const ofetch = window.fetch;
  if (ofetch) {{
    window.fetch = function(res, init) {{
      const url = (typeof res === 'string') ? res : (res && res.url) || '';
      if (!IGNORE.test(url)) window.__ankabot.pending++;
      return ofetch.apply(this, arguments)
        .finally(()=>{{ if (!IGNORE.test(url)) window.__ankabot.pending--; }});
    }}
  }}
  const oopen = XMLHttpRequest.prototype.open;
  XMLHttpRequest.prototype.open = function(m,u){{
    this.__ankabotURL = u || '';
    return oopen.apply(this, arguments);
  }};
  const osend = XMLHttpRequest.prototype.send;
  XMLHttpRequest.prototype.send = function(){{
    if (!IGNORE.test(this.__ankabotURL||'')) window.__ankabot.pending++;
    this.addEventListener('loadend', ()=>{{
      if (!IGNORE.test(this.__ankabotURL||'')) window.__ankabot.pending--;
    }}, {{ once:true }});
    return osend.apply(this, arguments);
  }};
}})();
"#,
        ignore
    )
}
fn wait_until_ready(
    tab: &headless_chrome::Tab,
    wait_ready: &str,
    network_idle_ms: u64,
    idle_threshold: u64,
    heuristic_min_chars: u64,
    deadline: Instant,
) -> Result<String> {
    let idle_dur = Duration::from_millis(network_idle_ms);
    let mut last_cnt: i64 = -1;
    let mut idle_since: Option<Instant> = None;
    let mut heur_since: Option<Instant> = None;

    if wait_ready.eq_ignore_ascii_case("none") {
        return Ok("ready_state".to_string());
    }

    loop {
        if Instant::now() >= deadline {
            return Err(anyhow!("wait_until_ready timeout"));
        }

        let ready_state = tab
            .evaluate("document.readyState", false)?
            .value
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_default();
        let ready_ok = match wait_ready {
            "interactive" => ready_state == "interactive" || ready_state == "complete",
            _ => ready_state == "complete",
        };
        if ready_ok {
            return Ok("ready_state".to_string());
        }

        let pending = tab
            .evaluate("window.__ankabot ? window.__ankabot.pending : 0", false)?
            .value
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let cnt = tab
            .evaluate("performance.getEntriesByType('resource').length", false)?
            .value
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let stable = cnt == last_cnt;
        if pending <= idle_threshold as i64 && stable {
            if idle_since.is_none() {
                idle_since = Some(Instant::now());
            }
            if Instant::now().duration_since(idle_since.unwrap()) >= idle_dur {
                return Ok("network_idle".to_string());
            }
        } else {
            idle_since = None;
        }
        last_cnt = cnt;

        let val = tab
            .evaluate(
                "(() => { const t = document.body ? document.body.innerText.length : 0; const h = !!document.querySelector('main,article,#app,#root'); return {t:t, h:h}; })()",
                false,
            )?
            .value
            .unwrap_or_else(|| serde_json::json!({}));
        let text_len = val.get("t").and_then(|v| v.as_u64()).unwrap_or(0);
        let has_main = val.get("h").and_then(|v| v.as_bool()).unwrap_or(false);
        if text_len >= heuristic_min_chars && has_main {
            if heur_since.is_none() {
                heur_since = Some(Instant::now());
            }
            if Instant::now().duration_since(heur_since.unwrap()) >= Duration::from_millis(600) {
                let settle_deadline =
                    std::cmp::min(deadline, Instant::now() + Duration::from_millis(800));
                let _ = wait_images_and_fonts(tab, settle_deadline);
                return Ok("heuristic".to_string());
            }
        } else {
            heur_since = None;
        }

        std::thread::sleep(Duration::from_millis(200));
    }
}

fn wait_for_selector(tab: &headless_chrome::Tab, sel: &str, deadline: Instant) -> Result<()> {
    while Instant::now() < deadline {
        if tab.find_element(sel).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    Err(anyhow!("selector '{}' not found before timeout", sel))
}

fn wait_images_and_fonts(tab: &headless_chrome::Tab, deadline: Instant) -> Result<()> {
    loop {
        if Instant::now() >= deadline {
            return Err(anyhow!("images/fonts timeout"));
        }
        let imgs_unloaded = tab
            .evaluate(
                "Array.from(document.images).filter(i=>!i.complete).length",
                false,
            )?
            .value
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let fonts_ready = tab
            .evaluate(
                "document.fonts ? document.fonts.status === 'loaded' : true",
                false,
            )?
            .value
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        if imgs_unloaded == 0 && fonts_ready {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn render_with_chrome(url: &str, paths: &RunPaths, args: &Cli) -> Result<RenderOutcome> {
    use headless_chrome::{
        protocol::cdp::Emulation::{
            SetDeviceMetricsOverride, SetFocusEmulationEnabled, SetGeolocationOverride,
            SetLocaleOverride, SetTimezoneOverride,
        },
        protocol::cdp::Page::{
            AddScriptToEvaluateOnNewDocument, BringToFront, CaptureScreenshotFormatOption,
            SetLifecycleEventsEnabled,
        },
        types::PrintToPdfOptions,
        Browser, LaunchOptionsBuilder,
    };
    use std::ffi::{OsStr, OsString};

    let user_dir = profile_dir(&args.profile, args.user_data_dir.clone());
    std::fs::create_dir_all(&user_dir)?;

    let (win_w, win_h) = args.window_size();

    let mut arg_vec: Vec<OsString> = vec![
        OsString::from("--disable-gpu"),
        OsString::from("--disable-dev-shm-usage"),
        OsString::from("--no-first-run"),
        OsString::from("--no-default-browser-check"),
        OsString::from("--hide-scrollbars"),
        OsString::from("--disable-blink-features=AutomationControlled"),
        OsString::from("--disable-background-timer-throttling"),
        OsString::from("--disable-renderer-backgrounding"),
        OsString::from("--disable-backgrounding-occluded-windows"),
        OsString::from(format!("--window-size={},{}", win_w, win_h)),
    ];
    if !args.headful {
        arg_vec.push(OsString::from("--headless=new"));
    }
    if let Some(p) = &args.proxy {
        arg_vec.push(OsString::from(format!("--proxy-server={}", p)));
    }
    if let Some(exts) = &args.extensions {
        arg_vec.push(OsString::from(format!("--load-extension={}", exts)));
        arg_vec.push(OsString::from(format!(
            "--disable-extensions-except={}",
            exts
        )));
    }
    if !args.no_virtual_time {
        arg_vec.push(OsString::from(format!(
            "--virtual-time-budget={}",
            args.max_wait_ms
        )));
    }

    let launch_opts = LaunchOptionsBuilder::default()
        .headless(!args.headful)
        .user_data_dir(Some(user_dir))
        .args(
            arg_vec
                .iter()
                .map(|s| s.as_os_str())
                .collect::<Vec<&OsStr>>(),
        )
        .build()
        .unwrap();

    let browser = Browser::new(launch_opts)?;
    let tab = browser.new_tab()?;

    tab.call_method(SetLifecycleEventsEnabled { enabled: true })?;

    tab.call_method(SetDeviceMetricsOverride {
        width: win_w,
        height: win_h,
        device_scale_factor: args.dpr,
        mobile: args.mobile,
        scale: None,
        screen_width: None,
        screen_height: None,
        position_x: None,
        position_y: None,
        dont_set_visible_size: None,
        screen_orientation: None,
        viewport: None,
        display_feature: None,
        device_posture: None,
    })?;

    let inject_js = build_instrument_js(args.idle_ignore.as_deref().unwrap_or(""));
    tab.call_method(AddScriptToEvaluateOnNewDocument {
        source: inject_js,
        world_name: None,
        include_command_line_api: None,
        run_immediately: Some(true),
    })?;

    let stealth_js = format!(
        r#"
(() => {{
  Object.defineProperty(navigator, 'webdriver', {{ get: () => undefined }});
  Object.defineProperty(document, 'hidden', {{ get: () => false }});
  Object.defineProperty(document, 'visibilityState', {{ get: () => 'visible' }});
  window.chrome = window.chrome || {{ runtime: {{}} }};
  Object.defineProperty(navigator, 'languages', {{ get: () => ['en-AE','en','ar-AE'] }});
  Object.defineProperty(navigator, 'plugins', {{ get: () => [1,2,3] }});
  const origQuery = window.navigator.permissions && window.navigator.permissions.query;
  if (origQuery) {{
    window.navigator.permissions.query = (p) =>
      p && p.name === 'notifications'
        ? Promise.resolve({{ state: Notification.permission }})
        : origQuery(p);
  }}
  const getD = (k, v) => Object.defineProperty(window, k, {{ get: () => v }});
  getD('outerWidth', {width});
  getD('outerHeight', {height});
}})();
"#,
        width = win_w,
        height = win_h
    );
    tab.call_method(AddScriptToEvaluateOnNewDocument {
        source: stealth_js,
        world_name: None,
        include_command_line_api: None,
        run_immediately: Some(true),
    })?;

    tab.set_user_agent(
        &ua_generator::ua::spoof_ua(),
        Some(&args.locale_or_default()),
        Some("Windows"),
    )?;
    if let Some(tz) = &args.tz {
        tab.call_method(SetTimezoneOverride {
            timezone_id: tz.clone(),
        })?;
    }
    if let Some(loc) = &args.locale {
        tab.call_method(SetLocaleOverride {
            locale: Some(loc.clone()),
        })?;
    }
    if let Some(g) = &args.geo {
        let parts: Vec<&str> = g.split(',').collect();
        if parts.len() >= 2 {
            if let (Ok(lat), Ok(lon)) = (parts[0].parse(), parts[1].parse()) {
                let acc = if parts.len() > 2 {
                    parts[2].parse().ok()
                } else {
                    None
                };
                tab.call_method(SetGeolocationOverride {
                    latitude: Some(lat),
                    longitude: Some(lon),
                    accuracy: acc,
                })?;
            }
        }
    }

    if let Some(p) = &args.import_cookies {
        let bytes = std::fs::read(p)?;
        let list: Vec<CookieJson> = serde_json::from_slice(&bytes)?;
        import_cookies_to_chrome(&tab, &list)?;
    }

    let start = Instant::now();
    let deadline = start + Duration::from_millis(args.max_wait_ms);

    let res: Result<ChromeRes> = (|| {
        tab.navigate_to(url)?;
        tab.wait_until_navigated()?;
        tab.call_method(BringToFront(None))?;
        tab.call_method(SetFocusEmulationEnabled { enabled: true })?;
        let wait_branch = wait_until_ready(
            &tab,
            &args.wait_ready,
            args.network_idle_ms,
            args.idle_threshold,
            args.heuristic_min_chars,
            deadline,
        )?;
        if let Some(sel) = &args.wait_selector {
            wait_for_selector(&tab, sel, deadline)?;
        }

        if let Some(p) = &args.export_cookies {
            let list = export_cookies_from_chrome(&tab)?;
            std::fs::write(p, serde_json::to_vec_pretty(&list)?)?;
        }

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
        std::fs::write(&paths.dom_html, &html)?;
        let final_url = tab.get_url();
        let redirected = final_url != url;

        let png = tab.capture_screenshot(CaptureScreenshotFormatOption::Png, None, None, true)?;
        std::fs::write(&paths.png, png)?;
        let screenshot_path = Some(paths.png.display().to_string());

        wait_images_and_fonts(&tab, deadline)?;
        let bytes = tab.print_to_pdf(Some(PrintToPdfOptions {
            print_background: Some(true),
            prefer_css_page_size: Some(true),
            margin_top: Some(0.0),
            margin_bottom: Some(0.0),
            margin_left: Some(0.0),
            margin_right: Some(0.0),
            ..Default::default()
        }))?;
        std::fs::write(&paths.pdf, &bytes)?;
        let pdf_saved = Some(paths.pdf.display().to_string());

        Ok(ChromeRes {
            final_url,
            status: None,
            redirected,
            html_path: paths.dom_html.display().to_string(),
            elapsed_ms: start.elapsed().as_millis() as u64,
            screenshot_path,
            pdf_path: pdf_saved,
            waf_detected: challenge,
            anti_bot_vendor: None,
            js_challenge: challenge,
            wait_branch,
        })
    })();

    match res {
        Ok(r) => Ok(RenderOutcome::Success(r)),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("timeout") || msg.contains("EventNeverCame") {
                let wait_branch = if msg.contains("network idle") {
                    "network_idle"
                } else if msg.contains("readyState") {
                    "ready_state"
                } else {
                    "heuristic"
                };

                let dom_text_chars = tab
                    .evaluate("document.body ? document.body.innerText.length : 0", false)?
                    .value
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let images_total = tab
                    .evaluate("document.images.length", false)?
                    .value
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let images_incomplete = tab
                    .evaluate(
                        "Array.from(document.images).filter(i=>!i.complete).length",
                        false,
                    )?
                    .value
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let pending_requests = tab
                    .evaluate("window.__ankabot ? window.__ankabot.pending : 0", false)?
                    .value
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                let ts = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
                let dbg_dir = args.debug_dir.join(&ts);
                std::fs::create_dir_all(&dbg_dir)?;
                let html_content = tab.get_content().unwrap_or_default();
                let html_path = dbg_dir.join("dom.html");
                let _ = std::fs::write(&html_path, html_content);
                let shot_path = dbg_dir.join("snap.png");
                if let Ok(png) =
                    tab.capture_screenshot(CaptureScreenshotFormatOption::Png, None, None, true)
                {
                    let _ = std::fs::write(&shot_path, png);
                }

                let host = url::Url::parse(url)
                    .ok()
                    .and_then(|u| u.host_str().map(|h| h.to_string()))
                    .unwrap_or_else(|| "page".to_string());
                let pdf_file = dbg_dir.join(format!("{host}.pdf"));
                let mut pdf_saved = None;
                if let Ok(bytes) = tab.print_to_pdf(Some(PrintToPdfOptions {
                    print_background: Some(true),
                    prefer_css_page_size: Some(true),
                    margin_top: Some(0.0),
                    margin_bottom: Some(0.0),
                    margin_left: Some(0.0),
                    margin_right: Some(0.0),
                    ..Default::default()
                })) {
                    if std::fs::write(&pdf_file, bytes).is_ok() {
                        pdf_saved = Some(pdf_file.display().to_string());
                    }
                }

                let report = TimeoutReport {
                    status: "timeout",
                    reason: msg,
                    url: url.to_string(),
                    deadline_ms: args.max_wait_ms,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    wait_branch: wait_branch.to_string(),
                    diagnostics: Diagnostics {
                        dom_text_chars,
                        images_total,
                        images_incomplete,
                        pending_requests,
                    },
                    artifacts: Artifacts {
                        html: html_path.display().to_string(),
                        screenshot: shot_path.display().to_string(),
                        pdf: pdf_saved,
                    },
                };
                Ok(RenderOutcome::Timeout(report))
            } else {
                Err(e)
            }
        }
    }
}

fn write_json<T: Serialize>(path: &Path, v: &T) -> Result<()> {
    std::fs::write(path, serde_json::to_string_pretty(v)?)?;
    println!("{}", path.display());
    Ok(())
}
