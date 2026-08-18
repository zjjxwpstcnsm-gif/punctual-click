use std::path::PathBuf;

use anyhow::{anyhow, Context as _, Result};
use crossbeam_channel::Sender;
use punctual_browser::{
    discover_browsers, BrowserDiscoveryOptions, BrowserInstallation, BrowserPage, BrowserSession,
    ClickDispatch, CompletionVerification, TargetProbe,
};
use punctual_core::{
    ClickAttemptGuard, CompletionSignal, EngineEvent, TargetCandidate, TargetFingerprint,
};
use tokio::sync::{Mutex, MutexGuard};
use url::Url;

use crate::EngineConfig;

/// Lazily owns the selected visible browser session used by both editor
/// inspection and scheduled execution. Browser discovery happens at startup,
/// while the actual browser process is started only when the first task needs
/// it. If the preferred browser cannot be automated, candidates are tried in
/// order until a usable backend is found.
pub(crate) struct BrowserHub {
    profile_root: PathBuf,
    installations: Vec<BrowserInstallation>,
    session: Mutex<Option<BrowserSession>>,
    events: Sender<EngineEvent>,
}

impl BrowserHub {
    pub(crate) fn new(config: &EngineConfig, events: Sender<EngineEvent>) -> Self {
        let inventory = discover_browsers(&BrowserDiscoveryOptions {
            explicit_executable: config.browser_executable.clone(),
            preference: config.browser_preference.clone(),
            resources_dir: config.resources_dir.clone(),
        });
        let selected = inventory
            .preferred()
            .map(|installation| installation.display_name().to_owned());
        let _ = events.send(EngineEvent::BrowserStateChanged {
            connected: false,
            browser_name: selected,
            message: inventory.summary_zh(),
        });

        Self {
            profile_root: config.profile_dir.clone(),
            installations: inventory.installations,
            session: Mutex::new(None),
            events,
        }
    }

    async fn session(&self) -> Result<MutexGuard<'_, Option<BrowserSession>>> {
        let mut guard = self.session.lock().await;
        if guard.is_some() {
            return Ok(guard);
        }
        if self.installations.is_empty() {
            let message = concat!(
                "没有检测到可自动控制的浏览器。macOS 可使用系统 Safari，",
                "Windows 可使用系统 Edge；也可以设置 PUNCTUAL_CHROMIUM 指向浏览器可执行文件"
            )
            .to_owned();
            let _ = self.events.send(EngineEvent::BrowserStateChanged {
                connected: false,
                browser_name: None,
                message: message.clone(),
            });
            return Err(anyhow!(message));
        }

        let mut failures = Vec::new();
        for installation in &self.installations {
            let browser_name = installation.display_name().to_owned();
            let _ = self.events.send(EngineEvent::BrowserStateChanged {
                connected: false,
                browser_name: Some(browser_name.clone()),
                message: format!("正在启动 {browser_name}…"),
            });

            match BrowserSession::launch(installation, &self.profile_root).await {
                Ok(session) => {
                    let profile_dir = installation.profile_dir(&self.profile_root);
                    let fallback_note = if failures.is_empty() {
                        String::new()
                    } else {
                        format!("；已自动跳过：{}", failures.join("、"))
                    };
                    let managed_note = if installation.is_managed {
                        "；这是应用内置的免安装兜底浏览器"
                    } else {
                        ""
                    };
                    let safari_note = if installation.kind == punctual_browser::BrowserKind::Safari
                    {
                        "；Safari 自动化窗口与日常浏览数据隔离"
                    } else {
                        ""
                    };
                    let _ = self.events.send(EngineEvent::BrowserStateChanged {
                        connected: true,
                        browser_name: Some(browser_name.clone()),
                        message: format!(
                            "已连接 {browser_name}；独立用户目录：{}{}{}{}",
                            profile_dir.display(),
                            managed_note,
                            safari_note,
                            fallback_note
                        ),
                    });
                    *guard = Some(session);
                    return Ok(guard);
                }
                Err(error) => {
                    tracing::warn!(
                        browser = %browser_name,
                        error = %format!("{error:#}"),
                        "browser candidate could not be launched"
                    );
                    failures.push(format!("{browser_name}（{error:#}）"));
                }
            }
        }

        let message = format!(
            "检测到浏览器，但均无法启动自动化会话：{}",
            failures.join("；")
        );
        let _ = self.events.send(EngineEvent::BrowserStateChanged {
            connected: false,
            browser_name: None,
            message: message.clone(),
        });
        Err(anyhow!(message)).context("failed to launch a supported browser session")
    }

    pub(crate) async fn open(&self, url: &Url) -> Result<BrowserPage> {
        let guard = self.session().await?;
        guard
            .as_ref()
            .context("browser session disappeared")?
            .open(url)
            .await
    }

    pub(crate) async fn detect_targets(&self, page: &BrowserPage) -> Result<Vec<TargetCandidate>> {
        let guard = self.session().await?;
        guard
            .as_ref()
            .context("browser session disappeared")?
            .detect_targets(page)
            .await
    }

    pub(crate) async fn highlight(
        &self,
        page: &BrowserPage,
        target: &TargetFingerprint,
    ) -> Result<bool> {
        let guard = self.session().await?;
        guard
            .as_ref()
            .context("browser session disappeared")?
            .highlight(page, target)
            .await
    }

    pub(crate) async fn prepare_target(
        &self,
        page: &BrowserPage,
        target: &TargetFingerprint,
    ) -> Result<TargetProbe> {
        let guard = self.session().await?;
        guard
            .as_ref()
            .context("browser session disappeared")?
            .prepare_target(page, target)
            .await
    }

    pub(crate) async fn probe_target(
        &self,
        page: &BrowserPage,
        target: &TargetFingerprint,
    ) -> Result<TargetProbe> {
        let guard = self.session().await?;
        guard
            .as_ref()
            .context("browser session disappeared")?
            .probe_target(page, target)
            .await
    }

    pub(crate) async fn click_once(
        &self,
        page: &BrowserPage,
        target: &TargetFingerprint,
        guard: &ClickAttemptGuard,
        signals: &[CompletionSignal],
    ) -> Result<ClickDispatch> {
        let session = self.session().await?;
        session
            .as_ref()
            .context("browser session disappeared")?
            .click_once(page, target, guard, signals)
            .await
    }

    pub(crate) async fn verify_completion(
        &self,
        page: &BrowserPage,
        dispatch: &ClickDispatch,
        signals: &[CompletionSignal],
    ) -> Result<CompletionVerification> {
        let guard = self.session().await?;
        guard
            .as_ref()
            .context("browser session disappeared")?
            .verify_completion(page, dispatch, signals)
            .await
    }

    pub(crate) async fn current_url(&self, page: &BrowserPage) -> Result<Option<Url>> {
        let guard = self.session().await?;
        guard
            .as_ref()
            .context("browser session disappeared")?
            .current_url(page)
            .await
    }

    pub(crate) async fn shutdown(&self) -> Result<()> {
        let session = self.session.lock().await.take();
        let browser_name = session
            .as_ref()
            .map(|session| session.browser_name().to_owned());
        if let Some(session) = session {
            session.close().await?;
        }
        let _ = self.events.send(EngineEvent::BrowserStateChanged {
            connected: false,
            browser_name,
            message: "浏览器会话已关闭".into(),
        });
        Ok(())
    }
}
