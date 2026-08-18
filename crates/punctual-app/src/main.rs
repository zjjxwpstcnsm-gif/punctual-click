mod dashboard;
mod editor;

use std::{fs, path::PathBuf, sync::Arc};

use anyhow::Result;
use directories::ProjectDirs;
use gpui::{px, size, AppContext as _, Application, WindowBounds, WindowOptions};
use gpui_component::Root;
use punctual_engine::{EngineConfig, EngineHandle};
use punctual_storage::SqliteTaskRepository;

use crate::dashboard::PunctualDashboard;

fn main() -> Result<()> {
    tracing_subscriber::fmt().init();

    let project_dirs = ProjectDirs::from("dev", "punctual", "Punctual")
        .expect("the operating system must expose a user data directory");
    fs::create_dir_all(project_dirs.data_dir())?;
    let database_path = project_dirs.data_dir().join("punctual.db");
    let repository = Arc::new(SqliteTaskRepository::open(database_path)?);

    let engine = EngineHandle::start(
        Arc::clone(&repository),
        EngineConfig {
            profile_dir: project_dirs.data_dir().join("browser-profile"),
            resources_dir: application_resources_dir(),
            ..EngineConfig::default()
        },
    )?;

    let app = Application::new();
    app.run(move |cx| {
        gpui_component::init(cx);

        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1280.0), px(820.0)), cx)),
            window_min_size: Some(size(px(960.0), px(640.0))),
            ..Default::default()
        };

        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        cx.open_window(window_options, move |window, cx| {
            let view = cx.new(|cx| PunctualDashboard::new(repository, engine, window, cx));
            cx.new(|cx| Root::new(view, window, cx))
        })
        .expect("failed to open the Punctual window");
    });
    Ok(())
}

fn application_resources_dir() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("PUNCTUAL_RESOURCES_DIR").map(PathBuf::from) {
        return Some(path);
    }
    let executable = std::env::current_exe().ok()?;
    #[cfg(target_os = "macos")]
    {
        let contents = executable.parent()?.parent()?;
        let resources = contents.join("Resources");
        return resources.is_dir().then_some(resources);
    }
    #[cfg(target_os = "windows")]
    {
        let resources = executable.parent()?.join("resources");
        return resources.is_dir().then_some(resources);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let resources = executable.parent()?.join("../share/punctual");
        resources.is_dir().then_some(resources)
    }
}
