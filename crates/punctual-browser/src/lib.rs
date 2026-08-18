//! Browser-facing target discovery and validation.
//!
//! The deterministic resolver works without a running browser and is covered
//! by unit tests. Enable the `cdp` feature to launch a visible Chromium through
//! `chromiumoxide` and execute the bundled scanner script.

#[cfg(feature = "cdp")]
mod automation;
#[cfg(feature = "cdp")]
mod chromium;
#[cfg(feature = "cdp")]
mod discovery;
mod locator;
mod manual;
mod resolver;
mod result;
mod scripts;
#[cfg(feature = "cdp")]
mod webdriver;

#[cfg(feature = "cdp")]
pub use automation::*;
#[cfg(feature = "cdp")]
pub use chromium::*;
#[cfg(feature = "cdp")]
pub use discovery::*;
pub use locator::*;
pub use manual::*;
pub use resolver::*;
pub use result::*;
pub use scripts::*;
#[cfg(feature = "cdp")]
pub use webdriver::*;
