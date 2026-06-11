//! VocalSync Studio - Rust backend entry point
//!
//! Handles audio I/O, pitch detection, DSP, file management, and settings persistence.
//! The Svelte frontend owns UI rendering.

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

            let engine = core::audio_engine::AudioEngine::new();
            app.manage(std::sync::Mutex::new(engine));

            let settings = core::settings::AppSettings::load_or_default();
            app.manage(std::sync::Mutex::new(settings));

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
            commands::audio_commands::update_runtime_latency,
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
            commands::audio_commands::estimate_system_latency,
            commands::audio_commands::calibrate_latency_rhythm_voice,
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
            commands::lyrics_commands::find_subtitle_files,
            commands::lyrics_commands::probe_embedded_subtitles,
            commands::lyrics_commands::extract_embedded_subtitle,
            commands::settings_commands::load_settings,
            commands::settings_commands::update_calibrated_latency,
            commands::settings_commands::update_mixer_settings,
            commands::settings_commands::update_pitch_engine,
            commands::settings_commands::update_show_startup_guide,
            commands::settings_commands::load_project_session,
            commands::settings_commands::save_project_session,
            commands::settings_commands::clear_project_session,
            commands::settings_commands::list_song_profiles,
            commands::settings_commands::save_song_profile,
            commands::settings_commands::load_song_profile,
            commands::settings_commands::rename_song_profile,
            commands::settings_commands::delete_song_profile,
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
