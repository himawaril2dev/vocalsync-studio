//! 統一錯誤型別，讓所有 Tauri Command 都能用 Result<T, AppError> 回傳。

use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("音訊引擎錯誤：{0}")]
    Audio(String),

    #[error("檔案操作錯誤：{0}")]
    Io(#[from] std::io::Error),

    #[error("WAV 格式錯誤：{0}")]
    Hound(#[from] hound::Error),

    #[error("設定錯誤：{0}")]
    Settings(String),

    #[error("內部錯誤：{0}")]
    Internal(String),
}

// Tauri 要求 Command 的錯誤型別實作 Serialize
impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
