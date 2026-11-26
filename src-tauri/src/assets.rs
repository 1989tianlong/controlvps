use std::time::Duration;

use chrono::Utc;
use keyring::Entry;
use reqwest::blocking::Client;
use serde::Deserialize;

use crate::db::Database;
use crate::error::{AppError, AppResult};
use crate::models::{
    AuthMethod, AuthProfile, AuthProfileInput, CloudAccount, CloudAccountInput, CloudChange,
    CloudChangeKind, CloudImportRequest, CloudImportResult, CloudPreview, CloudProvider,
    CloudResource, Server, ServerGroup, ServerGroupInput, ServerInput,
};
use crate::ssh;

const KEYCHAIN_SERVICE: &str = "com.controlvps.app";

pub fn create_account(db: &Database, input: &CloudAccountInput) -> AppResult<CloudAccount> {
    validate_account(input, true)?;
    let account = db.create_cloud_account(input)?;
    save_token(&account.id, input.token.as_deref())?;
    db.get_cloud_account(&account.id)
}

pub fn update_account(
    db: &Database,
    id: &str,
    input: &CloudAccountInput,
) -> AppResult<CloudAccount> {
    validate_account(input, false)?;
    db.update_cloud_account(id, input)?;
    if input
        .token
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        save_token(id, input.token.as_deref())?;
    }
    db.get_cloud_account(id)
}

pub fn delete_account(db: &Database, id: &str) -> AppResult<()> {
    db.ensure_cloud_account_deletable(id)?;
    db.delete_cloud_account(id)?;
    delete_token(id)
}

pub fn create_profile(db: &Database, input: &AuthProfileInput) -> AppResult<AuthProfile> {
    validate_profile(input, true)?;
    let profile = db.create_auth_profile(input)?;
    save_profile_secrets(&profile, input)?;
    db.get_auth_profile(&profile.id)
}

pub fn update_profile(db: &Database, id: &str, input: &AuthProfileInput) -> AppResult<AuthProfile> {
    validate_profile(input, false)?;
    db.update_auth_profile(id, input)?;
    let profile = db.get_auth_profile(id)?;
    save_profile_secrets(&profile, input)?;
    db.get_auth_profile(id)
}

pub fn delete_profile(db: &Database, id: &str) -> AppResult<()> {
    db.ensure_auth_profile_deletable(id)?;
    db.delete_auth_profile(id)?;
    delete_profile_secrets(id)
}

pub fn create_group(db: &Database, input: &ServerGroupInput) -> AppResult<ServerGroup> {
    db.create_server_group(input)
}

pub fn update_group(db: &Database, id: &str, input: &ServerGroupInput) -> AppResult<ServerGroup> {
    db.update_server_group(id, input)
}

pub fn preview(db: &Database, account_id: &str) -> AppResult<CloudPreview> {
    let account = db.get_cloud_account(account_id)?;
    let resources = fetch(&account)?;
    let servers = db.list_servers()?;
    let linked: Vec<&Server> = servers
        .iter()
        .filter(|server| {
            server.cloud_account_id.as_deref() == Some(account.id.as_str())
                && server.cloud_resource_id.is_some()
        })
        .collect();
    let mut changes = Vec::new();
    let mut unchanged = 0usize;
    for resource in &resources {
        match linked.iter().find(|server| {
            server.cloud_resource_id.as_deref() == Some(resource.resource_id.as_str())
        }) {
            None => changes.push(CloudChange {
                kind: CloudChangeKind::Added,
                resource: resource.clone(),
                server_id: None,
                summary: "云端存在，本地尚未导入".into(),
            }),
            Some(server) => {
                let differences = resource_differences(server, resource);
                if differences.is_empty() {
                    unchanged += 1;
                } else {
                    changes.push(CloudChange {
                        kind: CloudChangeKind::Changed,
                        resource: resource.clone(),
                        server_id: Some(server.id.clone()),
                        summary: differences.join("；"),
                    });
                }
            }
        }
    }
    for server in &linked {
        let resource_id = server.cloud_resource_id.as_deref().unwrap_or_default();
        if resources.iter().any(|item| item.resource_id == resource_id) {
            continue;
        }
        changes.push(CloudChange {
            kind: CloudChangeKind::Missing,
            resource: CloudResource {
                provider: account.provider,
                resource_id: resource_id.to_string(),
                name: server.name.clone(),
                host: server.host.clone(),
                region: server.region.clone(),
                status: "missing".into(),
                tags: server.tags.clone(),
                detail: "云端已不再返回这台机器".into(),
            },
            server_id: Some(server.id.clone()),
            summary: "云端已消失；本地 VPS 会保留，不会自动删除".into(),
        });
    }
    db.touch_cloud_account(account_id)?;
    let added = changes
        .iter()
        .filter(|item| item.kind == CloudChangeKind::Added)
        .count();
    let changed = changes
        .iter()
        .filter(|item| item.kind == CloudChangeKind::Changed)
        .count();
    let missing = changes
        .iter()
        .filter(|item| item.kind == CloudChangeKind::Missing)
        .count();
    Ok(CloudPreview {
        account_id: account.id,
        provider: account.provider,
        fetched_at: Utc::now().to_rfc3339(),
        added,
        changed,
        missing,
        unchanged,
        changes,
    })
}

pub fn import(db: &Database, request: &CloudImportRequest) -> AppResult<CloudImportResult> {
    if request.resource_ids.is_empty() || request.resource_ids.len() > 100 {
        return Err(AppError::Message("每次请选择 1 到 100 台云资产".into()));
    }
    let account = db.get_cloud_account(&request.account_id)?;
    let resources = fetch(&account)?;
    let group = match request
        .group_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(name) => Some(db.get_server_group_by_name(name)?),
        None => None,
    };
    let profile = match request.auth_profile_id.as_deref().or(group
        .as_ref()
        .and_then(|item| item.auth_profile_id.as_deref()))
    {
        Some(id) => Some(db.get_auth_profile(id)?),
        None => None,
    };
    let jump_server_id = request
        .jump_server_id
        .clone()
        .or_else(|| group.as_ref().and_then(|item| item.jump_server_id.clone()));
    if let Some(jump_id) = jump_server_id.as_deref() {
        db.get_server(jump_id)?;
    }
    let username = request
        .username
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            group
                .as_ref()
                .and_then(|item| item.default_username.clone())
        })
        .unwrap_or_else(|| "root".into());
    let port = request
        .port
        .or(group.as_ref().and_then(|item| item.default_port))
        .unwrap_or(22);
    if port == 0 {
        return Err(AppError::Message("SSH 端口必须在 1 到 65535 之间".into()));
    }
    let mut result = CloudImportResult {
        created: 0,
        updated: 0,
        skipped: 0,
    };
    for resource_id in &request.resource_ids {
        let Some(resource) = resources
            .iter()
            .find(|item| &item.resource_id == resource_id && !item.host.trim().is_empty())
        else {
            result.skipped += 1;
            continue;
        };
        if let Some(existing) = db.find_cloud_server(&account.id, resource_id)? {
            let mut input = server_to_input(&existing);
            input.name = resource.name.clone();
            input.host = resource.host.clone();
            input.region = resource.region.clone();
            input.tags = merge_tags(&existing.tags, &resource.tags);
            input.provider = Some(provider_label(account.provider).into());
            db.update_server(&existing.id, &input)?;
            result.updated += 1;
            continue;
        }
        let (auth_method, private_key_path, password, key_passphrase, auth_profile_id) =
            match &profile {
                Some(profile) => (
                    profile.auth_method.clone(),
                    profile.private_key_path.clone(),
                    ssh::profile_secret(&profile.id, "password")?,
                    ssh::profile_secret(&profile.id, "key-passphrase")?,
                    Some(profile.id.clone()),
                ),
                None => (
                    AuthMethod::Key,
                    Some("~/.ssh/id_ed25519".into()),
                    None,
                    None,
                    None,
                ),
            };
        let input = ServerInput {
            name: resource.name.clone(),
            host: resource.host.clone(),
            port,
            username: username.clone(),
            auth_method,
            private_key_path,
            password,
            key_passphrase,
            group_name: group.as_ref().map(|item| item.name.clone()).or_else(|| {
                request
                    .group_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            }),
            tags: resource.tags.clone(),
            provider: Some(provider_label(account.provider).into()),
            region: resource.region.clone(),
            notes: Some(format!("由云资产同步导入。{}", resource.detail)),
            cost: None,
            currency: Some("USD".into()),
            expires_at: None,
            jump_server_id: jump_server_id.clone(),
            auth_profile_id,
            cloud_account_id: Some(account.id.clone()),
            cloud_resource_id: Some(resource.resource_id.clone()),
        };
        let server = db.create_server(&input)?;
        ssh::save_credentials(&server, &input)?;
        result.created += 1;
    }
    db.touch_cloud_account(&account.id)?;
    Ok(result)
}

fn fetch(account: &CloudAccount) -> AppResult<Vec<CloudResource>> {
    let token = load_token(&account.id)?
        .ok_or_else(|| AppError::Message("Keychain 中没有这个云账户的访问令牌".into()))?;
    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|error| AppError::Message(format!("无法创建云 API 客户端：{error}")))?;
    match account.provider {
        CloudProvider::DigitalOcean => fetch_digitalocean(&client, &token),
        CloudProvider::Tailscale => fetch_tailscale(&client, &token),
    }
}

fn fetch_digitalocean(client: &Client, token: &str) -> AppResult<Vec<CloudResource>> {
    let mut resources = Vec::new();
    let mut url = format!(
        "{}/v2/droplets?per_page=200",
        api_base(
            "CONTROLVPS_DIGITALOCEAN_API",
            "https://api.digitalocean.com"
        )
    );
    for _ in 0..20 {
        let response = client
            .get(&url)
            .bearer_auth(token)
            .send()
            .map_err(|error| AppError::Message(format!("DigitalOcean 请求失败：{error}")))?;
        if !response.status().is_success() {
            return Err(AppError::Message(format!(
                "DigitalOcean 返回 HTTP {}",
                response.status().as_u16()
            )));
        }
        let page: DigitalOceanPage = response
            .json()
            .map_err(|error| AppError::Message(format!("无法解析 DigitalOcean 响应：{error}")))?;
        for droplet in page.droplets {
            let host = public_address(&droplet);
            let Some(host) = host else {
                resources.push(CloudResource {
                    provider: CloudProvider::DigitalOcean,
                    resource_id: droplet.id.to_string(),
                    name: droplet.name,
                    host: String::new(),
                    region: droplet.region.map(|region| region.slug),
                    status: droplet.status,
                    tags: droplet.tags,
                    detail: "没有公网地址，导入时会跳过".into(),
                });
                continue;
            };
            let detail = format!(
                "{} · {} vCPU · {} MiB",
                droplet
                    .image
                    .and_then(|image| image.distribution.or(image.slug))
                    .unwrap_or_else(|| "未知系统".into()),
                droplet.vcpus.unwrap_or(0),
                droplet.memory.unwrap_or(0)
            );
            resources.push(CloudResource {
                provider: CloudProvider::DigitalOcean,
                resource_id: droplet.id.to_string(),
                name: droplet.name,
                host,
                region: droplet.region.map(|region| region.slug),
                status: droplet.status,
                tags: droplet.tags,
                detail,
            });
        }
        match page
            .links
            .and_then(|links| links.pages)
            .and_then(|pages| pages.next)
        {
            Some(next) => url = next,
            None => break,
        }
    }
    Ok(resources)
}

fn fetch_tailscale(client: &Client, token: &str) -> AppResult<Vec<CloudResource>> {
    let response = client
        .get(format!(
            "{}/api/v2/tailnet/-/devices",
            api_base("CONTROLVPS_TAILSCALE_API", "https://api.tailscale.com")
        ))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .map_err(|error| AppError::Message(format!("Tailscale 请求失败：{error}")))?;
    if !response.status().is_success() {
        return Err(AppError::Message(format!(
            "Tailscale 返回 HTTP {}",
            response.status().as_u16()
        )));
    }
    let page: TailscalePage = response
        .json()
        .map_err(|error| AppError::Message(format!("无法解析 Tailscale 响应：{error}")))?;
    Ok(page
        .devices
        .into_iter()
        .filter_map(|device| {
            let host = device
                .addresses
                .iter()
                .find(|address| address.contains(':'))
                .or_else(|| device.addresses.first())
                .cloned()
                .filter(|address| !address.is_empty())?;
            let name = device
                .name
                .split('.')
                .next()
                .unwrap_or(device.name.as_str())
                .to_string();
            Some(CloudResource {
                provider: CloudProvider::Tailscale,
                resource_id: device.id,
                name,
                host,
                region: None,
                status: if device.online.unwrap_or(false) {
                    "online".into()
                } else {
                    "offline".into()
                },
                tags: device.tags.unwrap_or_default(),
                detail: device.hostname.unwrap_or_else(|| device.name.clone()),
            })
        })
        .collect())
}

fn public_address(droplet: &DigitalOceanDroplet) -> Option<String> {
    droplet.networks.as_ref().and_then(|networks| {
        networks
            .v4
            .iter()
            .find(|network| network.kind == "public")
            .or_else(|| networks.v6.iter().find(|network| network.kind == "public"))
            .map(|network| network.ip_address.clone())
    })
}

fn resource_differences(server: &Server, resource: &CloudResource) -> Vec<String> {
    let mut differences = Vec::new();
    if server.name != resource.name {
        differences.push(format!("名称 {} → {}", server.name, resource.name));
    }
    if !resource.host.is_empty() && server.host != resource.host {
        differences.push(format!("地址 {} → {}", server.host, resource.host));
    }
    if server.region != resource.region {
        differences.push("地区已变化".into());
    }
    differences
}

fn merge_tags(existing: &[String], incoming: &[String]) -> Vec<String> {
    let mut tags = existing.to_vec();
    for tag in incoming {
        if !tags.iter().any(|item| item == tag) {
            tags.push(tag.clone());
        }
    }
    tags
}

fn server_to_input(server: &Server) -> ServerInput {
    ServerInput {
        name: server.name.clone(),
        host: server.host.clone(),
        port: server.port,
        username: server.username.clone(),
        auth_method: server.auth_method.clone(),
        private_key_path: server.private_key_path.clone(),
        password: None,
        key_passphrase: None,
        group_name: server.group_name.clone(),
        tags: server.tags.clone(),
        provider: server.provider.clone(),
        region: server.region.clone(),
        notes: server.notes.clone(),
        cost: server.cost,
        currency: server.currency.clone(),
        expires_at: server.expires_at.clone(),
        jump_server_id: server.jump_server_id.clone(),
        auth_profile_id: server.auth_profile_id.clone(),
        cloud_account_id: server.cloud_account_id.clone(),
        cloud_resource_id: server.cloud_resource_id.clone(),
    }
}

fn api_base(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| value.starts_with("https://") || value.starts_with("http://127.0.0.1"))
        .unwrap_or_else(|| default.to_string())
}

fn provider_label(provider: CloudProvider) -> &'static str {
    match provider {
        CloudProvider::DigitalOcean => "DigitalOcean",
        CloudProvider::Tailscale => "Tailscale",
    }
}

fn validate_account(input: &CloudAccountInput, creating: bool) -> AppResult<()> {
    if input.name.trim().is_empty() || input.name.chars().count() > 80 {
        return Err(AppError::Message(
            "云账户名称不能为空且不能超过 80 个字符".into(),
        ));
    }
    if creating
        && input
            .token
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
    {
        return Err(AppError::Message("首次添加云账户需要访问令牌".into()));
    }
    if input
        .token
        .as_deref()
        .is_some_and(|value| value.len() > 4096)
    {
        return Err(AppError::Message("访问令牌过长".into()));
    }
    Ok(())
}

fn validate_profile(input: &AuthProfileInput, creating: bool) -> AppResult<()> {
    if input.name.trim().is_empty() || input.name.chars().count() > 80 {
        return Err(AppError::Message(
            "认证身份名称不能为空且不能超过 80 个字符".into(),
        ));
    }
    if input.auth_method == AuthMethod::Key
        && input
            .private_key_path
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
    {
        return Err(AppError::Message("私钥认证需要本机私钥路径".into()));
    }
    if creating
        && input.auth_method == AuthMethod::Password
        && input
            .password
            .as_deref()
            .is_none_or(|value| value.is_empty())
    {
        return Err(AppError::Message("首次创建密码身份时不能留空".into()));
    }
    Ok(())
}

fn save_profile_secrets(profile: &AuthProfile, input: &AuthProfileInput) -> AppResult<()> {
    match input.auth_method {
        AuthMethod::Password => {
            if let Some(password) = input.password.as_deref() {
                ssh::save_profile_secret(&profile.id, "password", password)?;
            }
            ssh::delete_profile_secret(&profile.id, "key-passphrase")?;
        }
        AuthMethod::Key => {
            if let Some(passphrase) = input.key_passphrase.as_deref() {
                ssh::save_profile_secret(&profile.id, "key-passphrase", passphrase)?;
            }
            ssh::delete_profile_secret(&profile.id, "password")?;
        }
        AuthMethod::Agent => {
            ssh::delete_profile_secret(&profile.id, "password")?;
            ssh::delete_profile_secret(&profile.id, "key-passphrase")?;
        }
    }
    Ok(())
}

fn delete_profile_secrets(id: &str) -> AppResult<()> {
    ssh::delete_profile_secret(id, "password")?;
    ssh::delete_profile_secret(id, "key-passphrase")
}

fn token_entry(account_id: &str) -> AppResult<Entry> {
    Entry::new(KEYCHAIN_SERVICE, &format!("cloud:{account_id}:token"))
        .map_err(|error| AppError::Keyring(error.to_string()))
}

fn save_token(account_id: &str, token: Option<&str>) -> AppResult<()> {
    let Some(token) = token.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    token_entry(account_id)?
        .set_password(token)
        .map_err(|error| AppError::Keyring(error.to_string()))
}

fn load_token(account_id: &str) -> AppResult<Option<String>> {
    match token_entry(account_id)?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(AppError::Keyring(error.to_string())),
    }
}

fn delete_token(account_id: &str) -> AppResult<()> {
    match token_entry(account_id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(AppError::Keyring(error.to_string())),
    }
}

#[derive(Deserialize)]
struct DigitalOceanPage {
    droplets: Vec<DigitalOceanDroplet>,
    links: Option<DigitalOceanLinks>,
}

#[derive(Deserialize)]
struct DigitalOceanLinks {
    pages: Option<DigitalOceanPages>,
}

#[derive(Deserialize)]
struct DigitalOceanPages {
    next: Option<String>,
}

#[derive(Deserialize)]
struct DigitalOceanDroplet {
    id: u64,
    name: String,
    memory: Option<u64>,
    vcpus: Option<u64>,
    status: String,
    region: Option<DigitalOceanRegion>,
    image: Option<DigitalOceanImage>,
    networks: Option<DigitalOceanNetworks>,
    tags: Vec<String>,
}

#[derive(Deserialize)]
struct DigitalOceanRegion {
    slug: String,
}

#[derive(Deserialize)]
struct DigitalOceanImage {
    distribution: Option<String>,
    slug: Option<String>,
}

#[derive(Deserialize)]
struct DigitalOceanNetworks {
    v4: Vec<DigitalOceanAddress>,
    #[serde(default)]
    v6: Vec<DigitalOceanAddress>,
}

#[derive(Deserialize)]
struct DigitalOceanAddress {
    ip_address: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct TailscalePage {
    devices: Vec<TailscaleDevice>,
}

#[derive(Deserialize)]
struct TailscaleDevice {
    id: String,
    name: String,
    hostname: Option<String>,
    addresses: Vec<String>,
    online: Option<bool>,
    tags: Option<Vec<String>>,
}
