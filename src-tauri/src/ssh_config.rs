use crate::error::{AppError, AppResult};

#[derive(Clone, Default)]
pub struct ConfigEntry {
    pub alias: String,
    pub hostname: Option<String>,
    pub username: Option<String>,
    pub port: Option<u16>,
    pub identity_file: Option<String>,
    pub unsupported: Option<String>,
}

pub fn parse(content: &str) -> AppResult<(Vec<ConfigEntry>, Vec<String>)> {
    let mut entries = Vec::new();
    let mut current = Vec::<ConfigEntry>::new();
    let mut warnings = Vec::new();
    for (index, line) in content.lines().enumerate() {
        let line_number = index + 1;
        let tokens = tokenize(line)?;
        if tokens.is_empty() {
            continue;
        }
        let (keyword, values) = if let Some((key, value)) = tokens[0].split_once('=') {
            (key.to_string(), value.to_string())
        } else {
            (tokens[0].clone(), tokens[1..].join(" "))
        };
        let keyword = keyword.to_ascii_lowercase();
        let values = values.trim();
        match keyword.as_str() {
            "host" => {
                entries.append(&mut current);
                current = values
                    .split_whitespace()
                    .filter(|alias| !alias.is_empty())
                    .map(|alias| ConfigEntry {
                        alias: alias.to_string(),
                        unsupported: contains_pattern(alias)
                            .then(|| "包含通配符，暂不导入".to_string()),
                        ..ConfigEntry::default()
                    })
                    .collect();
            }
            "match" => current.clear(),
            "include" => {
                warnings.push(format!("第 {line_number} 行 Include 不会自动展开"));
                for entry in &mut current {
                    entry.unsupported = Some("包含未展开的 Include 配置".into());
                }
            }
            _ => {
                for entry in &mut current {
                    match keyword.as_str() {
                        "hostname" => entry.hostname = Some(values.to_string()),
                        "user" => entry.username = Some(values.to_string()),
                        "port" => match values.parse::<u16>() {
                            Ok(0) | Err(_) => {
                                entry.unsupported = Some(format!("端口无效：{values}"))
                            }
                            Ok(port) => entry.port = Some(port),
                        },
                        "identityfile" => {
                            if entry.identity_file.is_some() {
                                entry.unsupported =
                                    Some("包含多个 IdentityFile，暂不确定认证顺序".into());
                            } else {
                                entry.identity_file = Some(values.to_string());
                            }
                        }
                        "proxyjump" | "proxycommand" if !values.eq_ignore_ascii_case("none") => {
                            entry.unsupported =
                                Some(format!("暂不支持 {keyword}，不会绕过代理直接连接"));
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    entries.append(&mut current);
    Ok((entries, warnings))
}

fn contains_pattern(value: &str) -> bool {
    value.contains(['*', '?', '!'])
}

fn tokenize(line: &str) -> AppResult<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    for character in line.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(active_quote) = quote {
            if character == active_quote {
                quote = None;
            } else {
                current.push(character);
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            '#' => break,
            character if character.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(character),
        }
    }
    if quote.is_some() || escaped {
        return Err(AppError::Message(
            "SSH Config 包含未闭合的引号或转义".into(),
        ));
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}
