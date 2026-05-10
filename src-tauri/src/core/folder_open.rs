use crate::error::AppError;
use std::path::Path;
use std::process::Command;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

pub fn open_folder(path: &Path) -> Result<(), AppError> {
    if !path.is_dir() {
        return Err(AppError::Internal(format!(
            "Folder does not exist: {}",
            path.display()
        )));
    }

    #[cfg(windows)]
    {
        let mut command = Command::new("explorer.exe");
        command.arg(path);
        command.creation_flags(CREATE_NO_WINDOW);
        command.spawn().map_err(AppError::Io)?;
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(path)
            .spawn()
            .map_err(AppError::Io)?;
        return Ok(());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(AppError::Io)?;
        return Ok(());
    }
}
