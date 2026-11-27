use std::collections::HashSet;
use std::io::Read;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, SecondsFormat, Utc};
use regex::Regex;
use reqwest::blocking::Client;
use serde::Serialize;

use crate::error::{AppError, AppResult};

const DEAL_CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_FEED_BYTES: u64 = 4 * 1024 * 1024;

fn deal_cache() -> &'static Mutex<Option<(Instant, DealFeed)>> {
    static CACHE: OnceLock<Mutex<Option<(Instant, DealFeed)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

struct FeedDefinition {
    id: &'static str,
    name: &'static str,
    url: &'static str,
}

const FEEDS: [FeedDefinition; 2] = [
    FeedDefinition {
        id: "lowendbox",
        name: "LowEndBox",
        url: "https://lowendbox.com/feed/",
    },
    FeedDefinition {
        id: "lowendtalk",
        name: "LowEndTalk 优惠区",
        url: "https://lowendtalk.com/categories/offers/feed.rss",
    },
];

const SKIP_HOSTS: &[&str] = &[
    "lowendbox.com",
    "lowendtalk.com",
    "twitter.com",
    "x.com",
    "facebook.com",
    "youtube.com",
    "youtu.be",
    "t.co",
    "imgur.com",
    "gravatar.com",
    "wordpress.com",
    "wp.com",
    "google.com",
    "goo.gl",
    "drive.google.com",
    "paypal.com",
    "paypal.me",
    "discord.gg",
    "discord.com",
    "t.me",
    "telegram.me",
    "linkedin.com",
    "instagram.com",
    "reddit.com",
    "github.com",
    "bit.ly",
    "tinyurl.com",
    "w3.org",
    "gstatic.com",
    "googleapis.com",
    "jsdelivr.net",
    "googletagmanager.com",
    "doubleclick.net",
    "cloudflare.com",
    "s.w.org",
];

const BRANDS: &[(&str, &str)] = &[
    ("racknerd", "RackNerd"),
    ("serverhost", "ServerHost"),
    ("dedirock", "DediRock"),
    ("fast moose", "Fast Moose"),
    ("fastmoose", "Fast Moose"),
    ("hetzner", "Hetzner"),
    ("contabo", "Contabo"),
    ("ovh", "OVHcloud"),
    ("vultr", "Vultr"),
    ("digitalocean", "DigitalOcean"),
    ("digital ocean", "DigitalOcean"),
    ("linode", "Akamai Linode"),
    ("akamai", "Akamai Linode"),
    ("hosthatch", "HostHatch"),
    ("buyvm", "BuyVM"),
    ("frantech", "BuyVM"),
    ("greencloud", "GreenCloud"),
    ("incognet", "Incognet"),
    ("alphavps", "AlphaVPS"),
    ("hostens", "Hostens"),
    ("ionos", "IONOS"),
    ("netcup", "netcup"),
    ("servarica", "Servarica"),
    ("crunchbits", "Crunchbits"),
    ("vimach", "VirMach"),
    ("hostbrr", "HostBrr"),
    ("hostodo", "Hostodo"),
    ("servury", "Servury"),
    ("deluxhost", "DeluxHost"),
    ("webhorizon", "WebHorizon"),
    ("cloudcone", "CloudCone"),
    ("hostcram", "HostCram"),
    ("pq.hosting", "PQ Hosting"),
    ("hostinger", "Hostinger"),
    ("kamatera", "Kamatera"),
    ("scaleway", "Scaleway"),
    ("oracle", "Oracle Cloud"),
    ("amazon", "AWS"),
    ("aws", "AWS"),
    ("azure", "Azure"),
    ("google cloud", "Google Cloud"),
    ("cloudflare", "Cloudflare"),
    ("hostdare", "HostDare"),
    ("bandwagonhost", "BandwagonHost"),
    ("justhost", "JustHost"),
    ("megalayer", "MegaLayer"),
    ("vmiss", "VMISS"),
    ("kdatacenter", "Kdatacenter"),
    ("cloudie", "Cloudie"),
    ("txvps", "TXVPS"),
    ("colocrossing", "ColoCrossing"),
    ("servercheap", "Servercheap"),
    ("hostsailor", "HostSailor"),
    ("inet.ws", "INET.WS"),
    ("cometvps", "CometVPS"),
    ("enderhost", "EnderHost"),
];

const LOCATIONS: &[(&str, &str)] = &[
    ("los angeles", "洛杉矶"),
    ("new york", "纽约"),
    ("dallas", "达拉斯"),
    ("chicago", "芝加哥"),
    ("seattle", "西雅图"),
    ("miami", "迈阿密"),
    ("atlanta", "亚特兰大"),
    ("ashburn", "阿什本"),
    ("phoenix", "凤凰城"),
    ("salt lake", "盐湖城"),
    ("las vegas", "拉斯维加斯"),
    ("san jose", "圣何塞"),
    ("silicon valley", "硅谷"),
    ("toronto", "多伦多"),
    ("montreal", "蒙特利尔"),
    ("vancouver", "温哥华"),
    ("mexico", "墨西哥"),
    ("sao paulo", "圣保罗"),
    ("london", "伦敦"),
    ("manchester", "曼彻斯特"),
    ("amsterdam", "阿姆斯特丹"),
    ("netherlands", "荷兰"),
    ("frankfurt", "法兰克福"),
    ("germany", "德国"),
    ("falkenstein", "法尔肯施泰因"),
    ("nuremberg", "纽伦堡"),
    ("paris", "巴黎"),
    ("france", "法国"),
    ("zurich", "苏黎世"),
    ("switzerland", "瑞士"),
    ("stockholm", "斯德哥尔摩"),
    ("sweden", "瑞典"),
    ("warsaw", "华沙"),
    ("poland", "波兰"),
    ("bucharest", "布加勒斯特"),
    ("romania", "罗马尼亚"),
    ("sofia", "索非亚"),
    ("bulgaria", "保加利亚"),
    ("moscow", "莫斯科"),
    ("russia", "俄罗斯"),
    ("kyiv", "基辅"),
    ("ukraine", "乌克兰"),
    ("istanbul", "伊斯坦布尔"),
    ("dubai", "迪拜"),
    ("singapore", "新加坡"),
    ("hong kong", "香港"),
    ("taipei", "台北"),
    ("seoul", "首尔"),
    ("tokyo", "东京"),
    ("osaka", "大阪"),
    ("japan", "日本"),
    ("mumbai", "孟买"),
    ("delhi", "德里"),
    ("india", "印度"),
    ("sydney", "悉尼"),
    ("melbourne", "墨尔本"),
    ("australia", "澳大利亚"),
    ("auckland", "奥克兰"),
    ("johannesburg", "约翰内斯堡"),
    ("south africa", "南非"),
    ("israel", "以色列"),
    ("brazil", "巴西"),
    ("chile", "智利"),
    ("argentina", "阿根廷"),
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DealFeed {
    pub fetched_at: String,
    pub sources: Vec<DealSource>,
    pub deals: Vec<Deal>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DealSource {
    pub id: String,
    pub name: String,
    pub url: String,
    pub ok: bool,
    pub message: Option<String>,
    pub item_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Deal {
    pub id: String,
    pub source: String,
    pub source_name: String,
    pub title: String,
    pub url: String,
    pub published_at: String,
    pub provider: Option<String>,
    pub summary: String,
    pub price: Option<DealPrice>,
    pub memory_gb: Option<f64>,
    pub disk_gb: Option<f64>,
    pub cpu_cores: Option<f64>,
    pub bandwidth_tb: Option<f64>,
    pub locations: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DealPrice {
    pub amount: f64,
    pub currency: String,
    pub period: String,
    pub billing_months: f64,
    pub monthly_equivalent: f64,
}

struct Patterns {
    html_tag: Regex,
    html_block: Regex,
    entity: Regex,
    whitespace: Regex,
    post_tail: Regex,
    price_usd: Regex,
    price_eur: Regex,
    price_for: Regex,
    memory_a: Regex,
    memory_b: Regex,
    disk_a: Regex,
    disk_b: Regex,
    cpu: Regex,
    bandwidth_a: Regex,
    bandwidth_b: Regex,
    link: Regex,
}

fn patterns() -> &'static Patterns {
    static PATTERNS: OnceLock<Patterns> = OnceLock::new();
    PATTERNS.get_or_init(|| Patterns {
        html_tag: Regex::new(r"(?is)<[^>]+>").expect("valid html tag pattern"),
        html_block: Regex::new(r"(?is)<script[^>]*>.*?</script>|<style[^>]*>.*?</style>")
            .expect("valid html block pattern"),
        entity: Regex::new(r"&(?:#\d+|#x[0-9a-fA-F]+|[a-zA-Z][a-zA-Z0-9]*);")
            .expect("valid entity pattern"),
        whitespace: Regex::new(r"\s+").expect("valid whitespace pattern"),
        post_tail: Regex::new(r"(?is)\s*The post .*?appeared first on [^.]*\.").expect("valid tail"),
        price_usd: Regex::new(
            r"(?ix)(?:US\s*)?\$\s?(?P<amount>\d{1,6}(?:\.\d{1,2})?)\s?(?:/|per\s?)?\s?(?P<period>mo|mth|month(?:ly)?|yr|year(?:ly)?|annum|annually|quarter(?:ly)?)\b",
        )
        .expect("valid usd price pattern"),
        price_eur: Regex::new(
            r"(?ix)€\s?(?P<amount>\d{1,6}(?:\.\d{1,2})?)\s?(?:/|per\s?)?\s?(?P<period>mo|mth|month(?:ly)?|yr|year(?:ly)?|annum|annually|quarter(?:ly)?)\b",
        )
        .expect("valid eur price pattern"),
        price_for: Regex::new(
            r"(?ix)\$\s?(?P<amount>\d{1,6}(?:\.\d{1,2})?)\s*for\s+(?P<count>\d{1,2})\s*(?P<unit>years?|yrs?|months?|mos?)\b",
        )
        .expect("valid bundle price pattern"),
        memory_a: Regex::new(r"(?i)(\d{1,3}(?:\.\d)?)\s*GB\s*(?:of\s+)?(?:RAM|memory)\b")
            .expect("valid memory pattern"),
        memory_b: Regex::new(r"(?i)(?:RAM|memory)\s*[:=]?\s*(\d{1,3}(?:\.\d)?)\s*GB")
            .expect("valid memory fallback pattern"),
        disk_a: Regex::new(
            r"(?i)(\d{1,4}(?:\.\d)?)\s*(GB|TB)\s*(?:SSD|NVMe|HDD|SAS|SATA|RAID|storage|disk|space)\b",
        )
        .expect("valid disk pattern"),
        disk_b: Regex::new(
            r"(?i)(?:SSD|NVMe|HDD|disk|storage|space)\s*[:=]?\s*(\d{1,4}(?:\.\d)?)\s*(GB|TB)",
        )
        .expect("valid disk fallback pattern"),
        cpu: Regex::new(r"(?i)(\d{1,3})\s*(?:x\s*)?(?:v?cpus?|v?cores?|core|threads?)\b")
            .expect("valid cpu pattern"),
        bandwidth_a: Regex::new(
            r"(?i)(\d{1,4}(?:\.\d)?)\s*(TB|GB)\s*(?:of\s+)?(?:premium\s+)?(?:bandwidth|transfer|traffic)\b",
        )
        .expect("valid bandwidth pattern"),
        bandwidth_b: Regex::new(
            r"(?i)(?:bandwidth|transfer|traffic)\s*[:=]?\s*(\d{1,4}(?:\.\d)?)\s*(TB|GB)\b",
        )
        .expect("valid bandwidth fallback pattern"),
        link: Regex::new(r#"(?i)href\s*=\s*["'](https?://[^"']+)["']"#).expect("valid link pattern"),
    })
}

pub fn fetch_deals() -> AppResult<DealFeed> {
    if let Ok(guard) = deal_cache().lock() {
        if let Some((fetched_at, feed)) = guard.as_ref() {
            if fetched_at.elapsed() < DEAL_CACHE_TTL {
                return Ok(feed.clone());
            }
        }
    }
    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(8))
        .user_agent(concat!(
            "ControlVPS/",
            env!("CARGO_PKG_VERSION"),
            " (+https://github.com/1989tianlong/controlvps)"
        ))
        .build()
        .map_err(|error| AppError::Message(format!("无法创建优惠抓取客户端：{error}")))?;

    let mut sources = Vec::new();
    let mut deals = Vec::new();
    let mut seen = HashSet::new();
    for feed in FEEDS.iter() {
        match fetch_feed(&client, feed, &mut seen) {
            Ok(mut items) => {
                sources.push(DealSource {
                    id: feed.id.into(),
                    name: feed.name.into(),
                    url: feed.url.into(),
                    ok: true,
                    message: None,
                    item_count: items.len(),
                });
                deals.append(&mut items);
            }
            Err(message) => sources.push(DealSource {
                id: feed.id.into(),
                name: feed.name.into(),
                url: feed.url.into(),
                ok: false,
                message: Some(message),
                item_count: 0,
            }),
        }
    }

    if deals.is_empty() {
        let reasons: Vec<String> = sources
            .iter()
            .filter_map(|source| source.message.clone())
            .collect();
        if !reasons.is_empty() {
            return Err(AppError::Message(reasons.join("；")));
        }
    }

    deals.sort_by(|a, b| b.published_at.cmp(&a.published_at));
    let feed = DealFeed {
        fetched_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        sources,
        deals,
    };
    if let Ok(mut guard) = deal_cache().lock() {
        *guard = Some((Instant::now(), feed.clone()));
    }
    Ok(feed)
}

fn fetch_feed(
    client: &Client,
    feed: &FeedDefinition,
    seen: &mut HashSet<String>,
) -> Result<Vec<Deal>, String> {
    let response = client
        .get(feed.url)
        .send()
        .map_err(|error| format!("{} 请求失败：{error}", feed.name))?;
    if !response.status().is_success() {
        return Err(format!(
            "{} 返回 HTTP {}",
            feed.name,
            response.status().as_u16()
        ));
    }
    let mut raw_bytes = Vec::new();
    response
        .take(MAX_FEED_BYTES)
        .read_to_end(&mut raw_bytes)
        .map_err(|error| format!("{} 内容读取失败：{error}", feed.name))?;
    let body = String::from_utf8_lossy(&raw_bytes);

    let mut items = Vec::new();
    for raw in feed_items(&body).into_iter().take(60) {
        let Some(deal) = parse_item(feed, raw) else {
            continue;
        };
        if !seen.insert(deal.url.clone()) {
            continue;
        }
        items.push(deal);
    }
    if items.is_empty() {
        return Err(format!("{} 没有解析到可用的优惠条目", feed.name));
    }
    Ok(items)
}

fn feed_items(body: &str) -> Vec<&str> {
    let mut items = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find("<item>") {
        let after = &rest[start + "<item>".len()..];
        let Some(end) = after.find("</item>") else {
            break;
        };
        items.push(&after[..end]);
        rest = &after[end + "</item>".len()..];
    }
    items
}

fn tag_text<'a>(item: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = item.find(&open)? + open.len();
    let end = item[start..].find(&close)? + start;
    Some(item[start..end].trim())
}

fn cdata(value: &str) -> &str {
    let trimmed = value.trim();
    if let Some(inner) = trimmed.strip_prefix("<![CDATA[") {
        if let Some(end) = inner.rfind("]]>") {
            return inner[..end].trim();
        }
    }
    trimmed
}

fn parse_item(feed: &FeedDefinition, item: &str) -> Option<Deal> {
    let title = inline_text(tag_text(item, "title")?);
    if title.is_empty() {
        return None;
    }
    let link = cdata(tag_text(item, "link")?).to_string();
    let Ok(parsed_link) = reqwest::Url::parse(&link) else {
        return None;
    };
    if !matches!(parsed_link.scheme(), "http" | "https") {
        return None;
    }
    let published_at = tag_text(item, "pubDate")
        .map(normalize_date)
        .unwrap_or_default();
    let description = tag_text(item, "description").unwrap_or("");
    let encoded = tag_text(item, "content:encoded").unwrap_or("");
    let body_text = plain_text(description);
    let encoded_text = plain_text(encoded);
    let picked = if encoded_text.chars().count() > body_text.chars().count() {
        &encoded_text
    } else {
        &body_text
    };
    let full_text = truncate(picked, 6000);
    let summary = truncate(&full_text, 320);

    let haystack = format!("{title}\n{full_text}");
    let spec_text = truncate(&haystack, 900);
    let provider = brand_in(&title)
        .or_else(|| provider_domain(encoded))
        .or_else(|| provider_domain(description));

    Some(Deal {
        id: format!("{}:{}", feed.id, short_hash(&link)),
        source: feed.id.into(),
        source_name: feed.name.into(),
        title,
        url: link,
        published_at,
        provider,
        summary,
        price: parse_price(&haystack),
        memory_gb: first_measure(&[&patterns().memory_a, &patterns().memory_b], &spec_text),
        disk_gb: parse_disk(&spec_text),
        cpu_cores: first_measure(&[&patterns().cpu], &spec_text),
        bandwidth_tb: parse_bandwidth(&spec_text),
        locations: locations_in(&spec_text),
    })
}

fn parse_price(text: &str) -> Option<DealPrice> {
    let patterns = patterns();
    let mut best: Option<DealPrice> = None;
    let mut consider = |price: DealPrice| {
        let replace = best
            .as_ref()
            .is_none_or(|current| price.monthly_equivalent < current.monthly_equivalent);
        if replace {
            best = Some(price);
        }
    };

    for cap in patterns.price_usd.captures_iter(text) {
        let amount = parse_number(&cap["amount"]);
        let months = period_months(&cap["period"]);
        consider(build_price(amount, "USD", months));
    }
    for cap in patterns.price_eur.captures_iter(text) {
        let amount = parse_number(&cap["amount"]);
        let months = period_months(&cap["period"]);
        consider(build_price(amount, "EUR", months));
    }
    for cap in patterns.price_for.captures_iter(text) {
        let amount = parse_number(&cap["amount"]);
        let count = parse_number(&cap["count"]).max(1.0);
        let unit = cap["unit"].to_ascii_lowercase();
        let months = if unit.starts_with("y") {
            count * 12.0
        } else {
            count
        };
        consider(build_price(amount, "USD", months));
    }
    best
}

fn build_price(amount: f64, currency: &str, months: f64) -> DealPrice {
    let months = if months <= 0.0 { 1.0 } else { months };
    let rounded = months.round();
    let period = match rounded as i64 {
        1 => "month",
        3 => "quarter",
        12 => "year",
        value if value > 12 => "multi_year",
        _ => "custom",
    };
    DealPrice {
        amount: round2(amount),
        currency: currency.into(),
        period: period.into(),
        billing_months: rounded,
        monthly_equivalent: round2(amount / months),
    }
}

fn period_months(period: &str) -> f64 {
    let value = period.to_ascii_lowercase();
    if value.starts_with('y') || value.starts_with("annual") || value.starts_with("annum") {
        12.0
    } else if value.starts_with('q') {
        3.0
    } else {
        1.0
    }
}

fn first_measure(patterns: &[&Regex], text: &str) -> Option<f64> {
    for pattern in patterns {
        if let Some(cap) = pattern.captures(text) {
            let value = parse_number(cap.get(1)?.as_str());
            if value > 0.0 {
                return Some(value);
            }
        }
    }
    None
}

fn parse_disk(text: &str) -> Option<f64> {
    let patterns = patterns();
    for pattern in [&patterns.disk_a, &patterns.disk_b] {
        if let Some(cap) = pattern.captures(text) {
            let value = parse_number(cap.get(1)?.as_str());
            if value <= 0.0 {
                continue;
            }
            let unit = cap.get(2).map(|value| value.as_str()).unwrap_or("GB");
            return Some(if unit.eq_ignore_ascii_case("TB") {
                value * 1024.0
            } else {
                value
            });
        }
    }
    None
}

fn parse_bandwidth(text: &str) -> Option<f64> {
    let patterns = patterns();
    for pattern in [&patterns.bandwidth_a, &patterns.bandwidth_b] {
        if let Some(cap) = pattern.captures(text) {
            let value = parse_number(cap.get(1)?.as_str());
            if value <= 0.0 {
                continue;
            }
            let unit = cap.get(2).map(|value| value.as_str()).unwrap_or("TB");
            return Some(if unit.eq_ignore_ascii_case("GB") {
                round2(value / 1024.0)
            } else {
                value
            });
        }
    }
    None
}

fn locations_in(text: &str) -> Vec<String> {
    let lowered = text.to_ascii_lowercase();
    let mut found: Vec<String> = Vec::new();
    for (needle, label) in LOCATIONS {
        if !lowered.contains(needle) {
            continue;
        }
        if found.iter().any(|existing| existing == label) {
            continue;
        }
        found.push((*label).to_string());
        if found.len() >= 4 {
            break;
        }
    }
    found
}

fn brand_in(title: &str) -> Option<String> {
    let lowered = title.to_ascii_lowercase();
    BRANDS
        .iter()
        .find(|(needle, _)| lowered.contains(needle))
        .map(|(_, label)| (*label).to_string())
}

fn provider_domain(html: &str) -> Option<String> {
    if html.is_empty() {
        return None;
    }
    let patterns = patterns();
    for cap in patterns.link.captures_iter(html) {
        let Some(raw) = cap.get(1) else {
            continue;
        };
        let Ok(url) = reqwest::Url::parse(raw.as_str()) else {
            continue;
        };
        let Some(host) = url.host_str() else {
            continue;
        };
        let host = host.trim_start_matches("www.").to_ascii_lowercase();
        if SKIP_HOSTS
            .iter()
            .any(|skip| host == *skip || host.ends_with(&format!(".{skip}")))
        {
            continue;
        }
        let labels: Vec<&str> = host.split('.').collect();
        if labels.len() < 2 {
            continue;
        }
        let root = if labels.len() >= 3
            && matches!(labels[labels.len() - 2], "co" | "com" | "net" | "org")
            && labels[labels.len() - 1].len() == 2
        {
            labels[labels.len() - 3]
        } else {
            labels[labels.len() - 2]
        };
        if root.len() < 2 || root.chars().all(|ch| ch.is_ascii_digit()) {
            continue;
        }
        let mut chars = root.chars();
        let Some(first) = chars.next() else {
            continue;
        };
        let mut label = first.to_uppercase().collect::<String>();
        label.push_str(chars.as_str());
        return Some(label);
    }
    None
}

fn normalize_date(raw: &str) -> String {
    DateTime::parse_from_rfc2822(raw.trim())
        .map(|value| {
            value
                .with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::Secs, true)
        })
        .unwrap_or_default()
}

fn inline_text(value: &str) -> String {
    let patterns = patterns();
    let decoded = decode_entities(cdata(value));
    patterns
        .whitespace
        .replace_all(decoded.trim(), " ")
        .into_owned()
}

fn plain_text(html: &str) -> String {
    let patterns = patterns();
    let no_blocks = patterns.html_block.replace_all(cdata(html), " ");
    let decoded = decode_entities(&no_blocks);
    let no_tags = patterns.html_tag.replace_all(&decoded, " ");
    let collapsed = patterns.whitespace.replace_all(&no_tags, " ");
    let trimmed = patterns.post_tail.replace(&collapsed, " ");
    patterns
        .whitespace
        .replace_all(trimmed.trim(), " ")
        .into_owned()
}

fn decode_entities(input: &str) -> String {
    let patterns = patterns();
    patterns
        .entity
        .replace_all(input, |caps: &regex::Captures| {
            let raw = caps.get(0).map(|value| value.as_str()).unwrap_or_default();
            let body = raw.trim_start_matches('&').trim_end_matches(';');
            if let Some(hex) = body
                .strip_prefix("#x")
                .or_else(|| body.strip_prefix("#X"))
            {
                return u32::from_str_radix(hex, 16)
                    .ok()
                    .and_then(char::from_u32)
                    .map(String::from)
                    .unwrap_or_else(|| raw.to_string());
            }
            if let Some(dec) = body.strip_prefix('#') {
                return dec
                    .parse::<u32>()
                    .ok()
                    .and_then(char::from_u32)
                    .map(String::from)
                    .unwrap_or_else(|| raw.to_string());
            }
            match body.to_ascii_lowercase().as_str() {
                "amp" => "&".into(),
                "lt" => "<".into(),
                "gt" => ">".into(),
                "quot" => "\"".into(),
                "apos" => "'".into(),
                "nbsp" => " ".into(),
                "mdash" => "—".into(),
                "ndash" => "–".into(),
                "hellip" => "…".into(),
                "rsquo" | "lsquo" => "'".into(),
                "ldquo" | "rdquo" => "\"".into(),
                "middot" => "·".into(),
                "euro" => "€".into(),
                _ => raw.to_string(),
            }
        })
        .into_owned()
}

fn truncate(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(limit).collect();
    out.push('…');
    out
}

fn parse_number(value: &str) -> f64 {
    value
        .replace(',', ".")
        .parse::<f64>()
        .map(round2)
        .unwrap_or(0.0)
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn short_hash(value: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:014x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_price_does_not_mislabel_usd_bundle_as_eur() {
        let text = "Special offer: $24 for 2 years! Normally €10/mo in Germany.";
        let price = parse_price(text).expect("price parsed");
        assert_eq!(price.currency, "USD");
        assert_eq!(price.billing_months, 24.0);
        assert_eq!(price.monthly_equivalent, 1.0);
    }

    #[test]
    fn parse_item_rejects_non_http_schemes() {
        let feed = &FEEDS[0];
        let bad = "<title>Promo</title><link>javascript:alert(1)</link>";
        assert!(parse_item(feed, bad).is_none());
        let bad2 = "<title>Promo</title><link>httpfoo://bar</link>";
        assert!(parse_item(feed, bad2).is_none());
        let good = "<title>RackNerd 2GB RAM</title><link>https://lowendbox.com/blog/test/</link><description>2GB RAM 30GB NVMe $18/yr in Los Angeles</description>";
        let deal = parse_item(feed, good).expect("valid deal");
        assert_eq!(deal.provider.as_deref(), Some("RackNerd"));
        assert_eq!(deal.memory_gb, Some(2.0));
        assert_eq!(deal.disk_gb, Some(30.0));
        assert!(deal.locations.contains(&"洛杉矶".to_string()));
    }
}
