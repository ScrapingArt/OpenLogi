//! Platform file picker abstraction for selecting applications or directories.
//!
//! Presents a modeless native file dialog (`NSOpenPanel`) on macOS defaulting to
//! `/Applications`, and delegates to GPUI's platform path prompt in headless tests
//! and on non-macOS platforms.

use std::path::PathBuf;

use gpui::App;
use tokio::sync::oneshot::Receiver;

#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2_app_kit::NSOpenPanel;
#[cfg(target_os = "macos")]
use objc2_foundation::{MainThreadMarker, NSString, NSURL};

/// Configure an `NSOpenPanel` for selecting an application bundle or folder,
/// defaulting to `/Applications`.
#[cfg(target_os = "macos")]
#[must_use]
pub fn configure_open_panel(mtm: MainThreadMarker) -> Retained<NSOpenPanel> {
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseDirectories(true);
    panel.setCanChooseFiles(true);
    panel.setAllowsMultipleSelection(false);
    let app_dir = NSString::from_str("/Applications");
    let app_url = NSURL::fileURLWithPath(&app_dir);
    panel.setDirectoryURL(Some(&app_url));
    panel
}

/// Prompt the user to select an application or folder.
///
/// On macOS in production, this presents a modeless `NSOpenPanel` initialized
/// to `/Applications`. In test environments and on non-macOS platforms, this
/// routes through GPUI's `cx.prompt_for_paths`.
#[must_use]
#[cfg(any(test, not(target_os = "macos")))]
pub fn prompt_for_application_or_folder(cx: &App) -> Receiver<Option<PathBuf>> {
    let prompt_rx = cx.prompt_for_paths(gpui::PathPromptOptions {
        files: true,
        directories: true,
        multiple: false,
        prompt: None,
    });
    let (tx, rx) = tokio::sync::oneshot::channel();
    cx.background_executor()
        .spawn(async move {
            let result = prompt_rx.await;
            let path = match result {
                Ok(Ok(Some(mut paths))) => paths.pop(),
                _ => None,
            };
            let _ = tx.send(path);
        })
        .detach();
    rx
}

/// Prompt the user to select an application or folder.
///
/// On macOS in production, this presents a modeless `NSOpenPanel` initialized
/// to `/Applications`. In test environments and on non-macOS platforms, this
/// routes through GPUI's `cx.prompt_for_paths`.
#[must_use]
#[cfg(all(target_os = "macos", not(test)))]
pub fn prompt_for_application_or_folder(cx: &App) -> Receiver<Option<PathBuf>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    if let Some(mtm) = MainThreadMarker::new() {
        let panel = configure_open_panel(mtm);
        let panel_for_handler = panel.clone();
        let tx = std::sync::Mutex::new(Some(tx));
        let handler = block2::RcBlock::new(move |response: objc2_app_kit::NSModalResponse| {
            let path = if response == objc2_app_kit::NSModalResponseOK {
                panel_for_handler
                    .URLs()
                    .firstObject()
                    .and_then(|url| url.path())
                    .map(|p| PathBuf::from(p.to_string()))
            } else {
                None
            };
            if let Ok(mut guard) = tx.lock() {
                if let Some(sender) = guard.take() {
                    let _ = sender.send(path);
                }
            }
        });
        panel.beginWithCompletionHandler(&handler);
    } else {
        let prompt_rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: true,
            multiple: false,
            prompt: None,
        });
        cx.background_executor()
            .spawn(async move {
                let result = prompt_rx.await;
                let path = match result {
                    Ok(Ok(Some(mut paths))) => paths.pop(),
                    _ => None,
                };
                let _ = tx.send(path);
            })
            .detach();
    }
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn configure_open_panel_mac() {
        let Some(mtm) = MainThreadMarker::new() else {
            // MainThreadMarker is only available on the AppKit main thread; cargo test
            // executes tests on worker threads where AppKit rejects panel construction.
            return;
        };
        let panel = configure_open_panel(mtm);
        assert!(panel.canChooseDirectories());
        assert!(panel.canChooseFiles());
        assert!(!panel.allowsMultipleSelection());
        let dir = panel.directoryURL().expect("directoryURL must be set");
        let path = dir.path().expect("directoryURL must have path");
        assert_eq!(path.to_string(), "/Applications");
    }

    #[gpui::test]
    fn prompt_for_application_or_folder_select(cx: &mut gpui::TestAppContext) {
        let mut rx = cx.update(|cx| prompt_for_application_or_folder(cx));
        assert!(cx.did_prompt_for_paths());
        cx.simulate_path_prompt_response(|options| {
            assert!(options.files);
            assert!(options.directories);
            assert!(!options.multiple);
            Some(vec![PathBuf::from("/Applications/Safari.app")])
        });
        cx.run_until_parked();
        let result = rx.try_recv().expect("receiver should have message");
        assert_eq!(result, Some(PathBuf::from("/Applications/Safari.app")));
    }

    #[gpui::test]
    fn prompt_for_application_or_folder_directory(cx: &mut gpui::TestAppContext) {
        let mut rx = cx.update(|cx| prompt_for_application_or_folder(cx));
        assert!(cx.did_prompt_for_paths());
        cx.simulate_path_prompt_response(|options| {
            assert!(options.files);
            assert!(options.directories);
            assert!(!options.multiple);
            Some(vec![PathBuf::from("/Applications/Utilities")])
        });
        cx.run_until_parked();
        let result = rx.try_recv().expect("receiver should have message");
        assert_eq!(result, Some(PathBuf::from("/Applications/Utilities")));
    }

    #[gpui::test]
    fn prompt_for_application_or_folder_cancel(cx: &mut gpui::TestAppContext) {
        let mut rx = cx.update(|cx| prompt_for_application_or_folder(cx));
        assert!(cx.did_prompt_for_paths());
        cx.simulate_path_prompt_response(|options| {
            assert!(options.files);
            assert!(options.directories);
            assert!(!options.multiple);
            None
        });
        cx.run_until_parked();
        let result = rx.try_recv().expect("receiver should have message");
        assert_eq!(result, None);
    }
}
