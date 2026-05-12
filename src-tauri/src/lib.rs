//! VocalSync Studio — Rust 後端入口
//!
//! 職責：音訊 I/O、音高偵測、DSP、檔案管理、設定持久化。
//! 前端（Svelte）負責所有 UI 渲染。

pub mod commands;
pub mod core;
pub mod error;
pub mod events;
pub mod security;

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            log::info!("VocalSync Studio starting...");

            // 初始化共享的 AudioEngine 狀態
            let engine = core::audio_engine::AudioEngine::new();
            app.manage(std::sync::Mutex::new(engine));

            // 初始化設定
            let settings = core::settings::AppSettings::load_or_default();
            app.manage(std::sync::Mutex::new(settings));

            if let Err(err) = core::whisper_engine::cleanup_whisper_cache() {
                log::warn!("[startup] failed to clean Whisper cache: {err}");
            }

            // YouTube 下載取消旗標
            app.manage(commands::download_commands::DownloadCancelFlag(
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            ));
            app.manage(commands::download_commands::DownloadRunFlag(
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            ));

            let webview_data_dir = core::portable_paths::ensure_dir("webview-data")?;
            WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
                .title("VocalSync Studio")
                .inner_size(1280.0, 800.0)
                .min_inner_size(900.0, 600.0)
                .resizable(true)
                .decorations(false)
                .data_directory(webview_data_dir)
                .build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::device_commands::list_devices,
            commands::audio_commands::load_backing,
            commands::audio_commands::clear_backing,
            commands::audio_commands::start_recording,
            commands::audio_commands::stop_recording,
            commands::audio_commands::clear_recording,
            commands::audio_commands::start_preview,
            commands::audio_commands::start_playback,
            commands::audio_commands::pause_playback,
            commands::audio_commands::seek,
            commands::audio_commands::set_volume,
            commands::audio_commands::load_guide_vocal,
            commands::audio_commands::clear_guide_vocal,
            commands::audio_commands::set_guide_vocal_offset,
            commands::audio_commands::set_guide_vocal_enabled,
            commands::audio_commands::export_audio,
            commands::audio_commands::get_pitch_track,
            commands::audio_commands::get_backing_pitch_track,
            commands::audio_commands::calibrate_latency,
            commands::audio_commands::set_loop_points,
            commands::audio_commands::clear_loop,
            commands::audio_commands::get_loop_points,
            commands::audio_commands::set_speed,
            commands::audio_commands::get_speed,
            commands::audio_commands::set_pitch_semitones,
            commands::audio_commands::get_pitch_semitones,
            commands::lyrics_commands::load_lyrics,
            commands::lyrics_commands::save_lyrics_as_lrc,
            commands::lyrics_commands::save_lyrics_as_subtitle,
            commands::lyrics_commands::auto_align_lyrics_to_audio,
            commands::lyrics_commands::align_lyrics_to_timed_transcript,
            commands::lyrics_commands::find_subtitle_files,
            commands::lyrics_commands::probe_embedded_subtitles,
            commands::lyrics_commands::extract_embedded_subtitle,
            commands::whisper_commands::check_whisper_tools,
            commands::whisper_commands::list_whisper_model_options,
            commands::whisper_commands::inspect_local_whisper_runner_path,
            commands::whisper_commands::trust_local_whisper_runner,
            commands::whisper_commands::inspect_local_whisper_model_path,
            commands::whisper_commands::trust_local_whisper_model,
            commands::whisper_commands::open_whisper_model_folder,
            commands::whisper_commands::install_whisper_runner,
            commands::whisper_commands::install_whisper_model,
            commands::whisper_commands::activate_installed_whisper_model,
            commands::whisper_commands::transcribe_vocals_with_whisper,
            commands::settings_commands::load_settings,
            commands::settings_commands::update_calibrated_latency,
            commands::settings_commands::update_mixer_settings,
            commands::settings_commands::update_pitch_engine,
            commands::settings_commands::update_show_startup_guide,
            commands::settings_commands::load_project_session,
            commands::settings_commands::save_project_session,
            commands::settings_commands::clear_project_session,
            commands::melody_commands::auto_detect_melody_source,
            commands::melody_commands::load_melody_from_path,
            commands::melody_commands::load_vocals_and_extract_melody,
            commands::melody_commands::auto_load_melody_for_backing,
            commands::melody_commands::align_audio_files,
            commands::melody_commands::open_crepe_model_folder,
            commands::download_commands::check_download_tools,
            commands::download_commands::detect_local_ffmpeg,
            commands::download_commands::detect_local_ytdlp,
            commands::download_commands::inspect_local_ytdlp_path,
            commands::download_commands::inspect_local_ffmpeg_path,
            commands::download_commands::detect_download_url_type,
            commands::download_commands::start_download,
            commands::download_commands::cancel_download,
            commands::download_commands::get_default_download_dir,
            commands::download_commands::install_ytdlp,
            commands::download_commands::install_ffmpeg,
            commands::download_commands::trust_local_ffmpeg,
            commands::download_commands::trust_local_ytdlp,
            commands::updates_commands::check_latest_release,
        ])
        .run(tauri::generate_context!())
        .expect("error while running VocalSync Studio");
}
