use std::{env, path::PathBuf, time::Duration};

use punctual_browser::ChromiumSession;
use url::Url;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let url = env::args()
        .nth(1)
        .unwrap_or_else(|| "https://example.com".into());
    let url = Url::parse(&url)?;
    let profile_dir = env::var_os("PUNCTUAL_PROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".punctual-browser-profile"));
    let executable = env::var_os("PUNCTUAL_CHROMIUM").map(PathBuf::from);

    let session = ChromiumSession::launch(profile_dir, executable.as_deref(), "Chromium").await?;
    let page = session.open(&url).await?;
    tokio::time::sleep(Duration::from_secs(2)).await;

    let candidates = session.detect_targets(&page).await?;
    if candidates.is_empty() {
        println!("没有发现按钮候选。请确认页面已经加载完成。");
    } else {
        for (index, candidate) in candidates.iter().take(20).enumerate() {
            println!(
                "{:>2}. {:>3}% {:<20} selector={:?} shadow={:?}",
                index + 1,
                candidate.confidence,
                candidate.best_name(),
                candidate.selector_hint,
                candidate.shadow_path,
            );
            for reason in &candidate.score_reasons {
                println!("    - {reason}");
            }
        }
    }

    println!("浏览器保持可见 10 秒，便于检查页面。按 Ctrl+C 可提前结束。");
    tokio::time::sleep(Duration::from_secs(10)).await;
    session.close().await?;
    Ok(())
}
