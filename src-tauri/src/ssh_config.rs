use crate::error::{AppError, AppResult};

#[derive(Clone, Default, Debug, PartialEq, Eq)]
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
    let mut global_defaults = ConfigEntry::default();
    let mut in_global_wildcard = false;
    let mut warnings = Vec::new();

    for (index, line) in content.lines().enumerate() {
        let line_number = index + 1;
        let tokens = tokenize(line)?;
        if tokens.is_empty() {
            continue;
        }
        let (keyword, values) = parse_directive(&tokens);
        let keyword = keyword.to_ascii_lowercase();
        let values = values.trim();
        match keyword.as_str() {
            "host" => {
                entries.append(&mut current);
                let aliases: Vec<&str> = values
                    .split_whitespace()
                    .filter(|alias| !alias.is_empty())
                    .collect();
                if aliases.len() == 1 && aliases[0] == "*" {
                    in_global_wildcard = true;
                    continue;
                }
                in_global_wildcard = false;
                current = aliases
                    .into_iter()
                    .map(|alias| ConfigEntry {
                        alias: alias.to_string(),
                        unsupported: contains_pattern(alias)
                            .then(|| "包含通配符，暂不导入".to_string()),
                        ..ConfigEntry::default()
                    })
                    .collect();
            }
            "match" => {
                entries.append(&mut current);
                in_global_wildcard = false;
            }
            "include" => {
                warnings.push(format!("第 {line_number} 行 Include 不会自动展开"));
                for entry in &mut current {
                    entry.unsupported = Some("包含未展开的 Include 配置".into());
                }
            }
            _ => {
                if in_global_wildcard {
                    apply_directive(&mut global_defaults, &keyword, values);
                } else {
                    for entry in &mut current {
                        apply_directive(entry, &keyword, values);
                    }
                }
            }
        }
    }
    entries.append(&mut current);

    // Apply `Host *` fallback defaults following OpenSSH first-match-wins semantics.
    for entry in &mut entries {
        if entry.unsupported.is_some() {
            continue;
        }
        if entry.username.is_none() {
            entry.username = global_defaults.username.clone();
        }
        if entry.port.is_none() {
            entry.port = global_defaults.port;
        }
        if entry.identity_file.is_none() {
            entry.identity_file = global_defaults.identity_file.clone();
        }
    }

    Ok((entries, warnings))
}

fn parse_directive(tokens: &[String]) -> (String, String) {
    if let Some((key, value)) = tokens[0].split_once('=') {
        let rest = if tokens.len() > 1 {
            format!("{value} {}", tokens[1..].join(" "))
        } else {
            value.to_string()
        };
        (key.to_string(), rest)
    } else if tokens.get(1).is_some_and(|token| token == "=") {
        (tokens[0].clone(), tokens[2..].join(" "))
    } else if let Some(rest) = tokens.get(1).and_then(|token| token.strip_prefix('=')) {
        let mut combined = rest.to_string();
        if tokens.len() > 2 {
            if !combined.is_empty() {
                combined.push(' ');
            }
            combined.push_str(&tokens[2..].join(" "));
        }
        (tokens[0].clone(), combined)
    } else {
        (tokens[0].clone(), tokens[1..].join(" "))
    }
}

fn apply_directive(entry: &mut ConfigEntry, keyword: &str, values: &str) {
    // OpenSSH ssh_config uses first-obtained-value-wins semantics.
    match keyword {
        "hostname" => {
            if entry.hostname.is_none() {
                entry.hostname = Some(values.to_string());
            }
        }
        "user" => {
            if entry.username.is_none() {
                entry.username = Some(values.to_string());
            }
        }
        "port" => {
            if entry.port.is_none() {
                match values.parse::<u16>() {
                    Ok(0) | Err(_) => entry.unsupported = Some(format!("端口无效：{values}")),
                    Ok(port) => entry.port = Some(port),
                }
            }
        }
        "identityfile" => {
            if entry.identity_file.is_some() {
                entry.unsupported = Some("包含多个 IdentityFile，暂不确定认证顺序".into());
            } else {
                entry.identity_file = Some(values.to_string());
            }
        }
        "proxyjump" | "proxycommand" if !values.eq_ignore_ascii_case("none") => {
            entry.unsupported = Some(format!("暂不支持 {keyword}，不会绕过代理直接连接"));
        }
        _ => {}
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_equals_spacing_first_match_wins_and_global_wildcard() {
        let cfg = r#"
Host prod
  HostName = 203.0.113.10
  HostName 198.51.100.2
  User deploy
  User root

Host *
  Port = 2222
  IdentityFile ~/.ssh/id_ed25519
"#;
        let (entries, warnings) = parse(cfg).expect("parse ok");
        assert!(warnings.is_empty());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].alias, "prod");
        assert_eq!(entries[0].hostname.as_deref(), Some("203.0.113.10"));
        assert_eq!(entries[0].username.as_deref(), Some("deploy"));
        assert_eq!(entries[0].port, Some(2222));
        assert_eq!(
            entries[0].identity_file.as_deref(),
            Some("~/.ssh/id_ed25519")
        );
    }
}
