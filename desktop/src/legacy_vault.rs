//! One-time, best-effort removal of the vault items the removed Zephium
//! account kept. Nothing reads them any more; failures are ignored.

/// Deletes the account bearer items from the OS vault, if any remain.
pub(crate) fn drop_account_items() {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    std::thread::spawn(|| {
        const UNSCOPED: &str = "app.zephium.account";
        const SCOPED: &str = "app.zephium.account|https://api.zephium.app";
        for item in [UNSCOPED, SCOPED] {
            let Ok(_turn) = zephium_credentials::turn() else {
                return;
            };
            if zephium_credentials::present(item).unwrap_or(false) {
                let _ = zephium_credentials::delete(item);
            }
        }
    });
}
