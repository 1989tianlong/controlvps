use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("数据库操作失败：{0}")]
    Database(#[from] rusqlite::Error),
    #[error("SSH 操作失败：{0}")]
    Ssh(#[from] ssh2::Error),
    #[error("系统操作失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("无法访问系统钥匙串：{0}")]
    Keyring(String),
    #[error("{0}")]
    Message(String),
    #[error("未知主机密钥")]
    UnknownHostKey { fingerprint: String },
    #[error("主机密钥已改变")]
    ChangedHostKey { fingerprint: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub code: String,
    pub message: String,
    pub fingerprint: Option<String>,
}

impl From<AppError> for CommandError {
    fn from(error: AppError) -> Self {
        let (code, fingerprint) = match &error {
            AppError::UnknownHostKey { fingerprint } => {
                ("HOST_KEY_UNKNOWN", Some(fingerprint.clone()))
            }
            AppError::ChangedHostKey { fingerprint } => {
                ("HOST_KEY_CHANGED", Some(fingerprint.clone()))
            }
            AppError::Keyring(_) => ("KEYCHAIN_ERROR", None),
            AppError::Database(_) => ("DATABASE_ERROR", None),
            AppError::Ssh(_) => ("SSH_ERROR", None),
            AppError::Io(_) => ("IO_ERROR", None),
            AppError::Message(_) => ("OPERATION_ERROR", None),
        };
        Self {
            code: code.to_string(),
            message: error.to_string(),
            fingerprint,
        }
    }
}

pub type AppResult<T> = Result<T, AppError>;
pub type CommandResult<T> = Result<T, CommandError>;
