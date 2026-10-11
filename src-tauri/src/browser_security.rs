//! Native browser hardening, scoped to the discovery view, never system-wide.
//! This is defense in depth, not an OS firewall or a sandbox-escape guarantee.
use std::net::SocketAddr;

pub(crate) const POLICY_ERROR: &str = "Не вдалося увімкнути захист браузера. Сторінку не відкрито. Оновіть систему або браузерний компонент і спробуйте ще раз.";

#[cfg(any(target_os = "macos", test))]
fn local_resource_rules() -> String {
    // CFNetwork can bypass even an explicit SOCKS proxy for literal loopback
    // destinations. Block these BEFORE a socket is opened, in WebKit's native
    // content-extension engine (all resource types, not injected JavaScript).
    // Conservatively disallow ALL numeric/IP-literal browser resources. Named
    // public hosts still go through the DNS-pinning relay; media downloads use
    // their independent guarded transport. WebKit canonicalizes IPv4 spellings.
    // WebKit rejects alternation and nested `$`; serialized web URLs have a
    // slash after the authority. Use a separate rule per suffix.
    let mut patterns = vec![
        r"^[a-z-]+://[0-9.]+[:/]".to_string(),
        r"^[a-z-]+://\[[0-9a-f:.]+\]".to_string(),
        r"^[a-z-]+://[^./:]+[:/]".to_string(),
        r"^[a-z-]+://[^/]*[@%]".to_string(),
    ];
    for suffix in [
        "localhost",
        "local",
        "internal",
        "home",
        "lan",
        "test",
        "invalid",
        "onion",
        "arpa",
    ] {
        patterns.push(format!(r"^[a-z-]+://([^/]*\.)?{suffix}\.?[:/]"));
    }
    serde_json::to_string(
        &patterns
            .iter()
            .map(|pattern| {
                serde_json::json!({
                    "trigger": {"url-filter": pattern}, "action": {"type": "block"}
                })
            })
            .collect::<Vec<_>>(),
    )
    .expect("static content rules")
}

pub(crate) fn preflight() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    for name in [
        "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
        "WEBVIEW2_USER_DATA_FOLDER",
        "WEBVIEW2_BROWSER_EXECUTABLE_FOLDER",
    ] {
        // Do not silently accept an environment override which can replace the
        // isolated profile/runtime or append a conflicting proxy switch.
        if std::env::var_os(name).is_some_and(|value| !value.is_empty()) {
            return Err(POLICY_ERROR.into());
        }
    }
    Ok(())
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn windows_arguments(address: SocketAddr) -> String {
    // Explicit loopback subtraction is essential: Chromium otherwise bypasses
    // a proxy for localhost/link-local. No DIRECT fallback, PAC or system proxy.
    // SOCKS5 in Chromium is TCP-only. WebRTC must not use non-proxied UDP;
    // QUIC/WebTransport are disabled. Keep SmartScreen/site isolation ENABLED.
    // WebView2 uses the normal Chromium switch. The headless-only `force-`
    // variant is ignored here and allowed STUN packets in the native smoke test.
    format!("--proxy-server=socks5://{address} --proxy-bypass-list=<-loopback> --webrtc-ip-handling-policy=disable_non_proxied_udp --disable-quic --disable-blink-features=WebTransport,DirectSockets --disable-features=WebTransport,DirectSockets --no-first-run")
}

pub(crate) async fn configure(
    window: &tauri::WebviewWindow,
    address: SocketAddr,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        window
            .with_webview(move |webview| {
                // Tauri dispatches this closure to the AppKit main thread. The view
                // contains only about:blank until this function succeeds.
                unsafe { configure_macos(webview.inner().cast(), address, tx) };
            })
            .map_err(|_| POLICY_ERROR)?;
        tokio::time::timeout(std::time::Duration::from_secs(10), rx)
            .await
            .map_err(|_| POLICY_ERROR)?
            .map_err(|_| POLICY_ERROR)??;
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        // The isolated WebView2 environment receives policy arguments BEFORE
        // creation, never after a remote page has already started loading.
        let _ = (window, address);
        Ok(())
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (window, address);
        Err(POLICY_ERROR.into())
    }
}

#[cfg(target_os = "macos")]
unsafe fn configure_macos(
    view: *mut objc2::runtime::AnyObject,
    address: SocketAddr,
    tx: tokio::sync::oneshot::Sender<Result<(), String>>,
) {
    use objc2::{
        msg_send,
        rc::Retained,
        runtime::{AnyClass, AnyObject},
    };
    use objc2_foundation::NSString;
    if let Err(error) = macos(view, address) {
        let _ = tx.send(Err(error));
        return;
    }
    let Some(class) = AnyClass::get(c"WKContentRuleListStore") else {
        let _ = tx.send(Err(POLICY_ERROR.into()));
        return;
    };
    let store: Retained<AnyObject> = msg_send![class, defaultStore];
    let configuration: Retained<AnyObject> = msg_send![view, configuration];
    let controller: Retained<AnyObject> = msg_send![&configuration, userContentController];
    let sender = std::cell::RefCell::new(Some(tx));
    let completion = block2::RcBlock::new(move |list: *mut AnyObject, error: *mut AnyObject| {
        let result = if error.is_null() && !list.is_null() {
            let _: () = msg_send![&controller, addContentRuleList: list];
            Ok(())
        } else {
            Err(POLICY_ERROR.into())
        };
        if let Some(tx) = sender.borrow_mut().take() {
            let _ = tx.send(result);
        }
    });
    let _: () = msg_send![&store,
        compileContentRuleListForIdentifier: &*NSString::from_str("baldoj-local-network-block-v1"),
        encodedContentRuleList: &*NSString::from_str(&local_resource_rules()),
        completionHandler: &*completion
    ];
}

#[cfg(target_os = "macos")]
unsafe fn macos(view: *mut objc2::runtime::AnyObject, address: SocketAddr) -> Result<(), String> {
    use objc2::{
        msg_send,
        rc::Retained,
        runtime::{AnyClass, AnyObject, Bool},
        sel,
    };
    use objc2_foundation::{NSArray, NSProcessInfo, NSString};
    use std::ffi::{c_char, c_void, CString};
    if NSProcessInfo::processInfo()
        .operatingSystemVersion()
        .majorVersion
        < 26
    {
        return Err("Захищений пошук у браузері потребує macOS 26 або новішої. Звичайне завантаження залишається доступним.".into());
    }
    let config: Retained<AnyObject> = msg_send![view, configuration];
    let store: Retained<AnyObject> = msg_send![&config, websiteDataStore];
    let preferences: Retained<AnyObject> = msg_send![&config, preferences];
    for (object, selector) in [
        (&*store, sel!(setProxyConfigurations:)),
        (&*store, sel!(proxyConfigurations)),
        (&*preferences, sel!(_setPeerConnectionEnabled:)),
        (&*preferences, sel!(_peerConnectionEnabled)),
        (&*preferences, sel!(_setEnabled:forFeature:)),
        (&*preferences, sel!(_isEnabledForFeature:)),
    ] {
        let supported: bool = msg_send![object, respondsToSelector: selector];
        if !supported {
            return Err(POLICY_ERROR.into());
        }
    }
    // WebKit exposes no public per-view WebRTC off switch. These guarded SPI
    // selectors must be present AND read back false, or the view stays blank.
    // This dependency is deliberately documented, not a silent KVC best effort.
    let _: () = msg_send![&preferences, _setPeerConnectionEnabled: false];
    let rtc: bool = msg_send![&preferences, _peerConnectionEnabled];
    if rtc {
        return Err(POLICY_ERROR.into());
    }
    let class = AnyClass::get(c"WKPreferences").ok_or(POLICY_ERROR)?;
    let supported: bool = msg_send![class, respondsToSelector: sel!(_features)];
    if !supported {
        return Err(POLICY_ERROR.into());
    }
    let features: Retained<NSArray<AnyObject>> = msg_send![class, _features];
    let mut found_transport = false;
    for feature in features.iter() {
        let key: Retained<NSString> = msg_send![&feature, key];
        if key.to_string() == "WebTransportEnabled" {
            let _: () = msg_send![&preferences, _setEnabled: false, forFeature: &*feature];
            let enabled: bool = msg_send![&preferences, _isEnabledForFeature: &*feature];
            if enabled {
                return Err(POLICY_ERROR.into());
            }
            found_transport = true;
        }
    }
    if !found_transport {
        return Err(POLICY_ERROR.into());
    }

    // Resolve macOS 14+ Network API dynamically: the rest of the application
    // remains launchable on older systems; discovery itself requires macOS 26.
    static NETWORK_LIBRARY: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let library = *NETWORK_LIBRARY.get_or_init(|| {
        libc::dlopen(
            c"/System/Library/Frameworks/Network.framework/Network".as_ptr(),
            libc::RTLD_LOCAL | libc::RTLD_LAZY,
        ) as usize
    }) as *mut c_void;
    if library.is_null() {
        return Err(POLICY_ERROR.into());
    }
    // Keep the system framework loaded for process lifetime: its returned
    // Objective-C objects can outlive this configuration callback.
    let endpoint_fn = libc::dlsym(library, c"nw_endpoint_create_host".as_ptr());
    let proxy_fn = libc::dlsym(library, c"nw_proxy_config_create_socksv5".as_ptr());
    let failover_fn = libc::dlsym(library, c"nw_proxy_config_set_failover_allowed".as_ptr());
    if endpoint_fn.is_null() || proxy_fn.is_null() || failover_fn.is_null() {
        return Err(POLICY_ERROR.into());
    }
    let endpoint_fn: unsafe extern "C" fn(*const c_char, *const c_char) -> *mut AnyObject =
        std::mem::transmute::<*mut c_void, _>(endpoint_fn);
    let proxy_fn: unsafe extern "C" fn(*mut AnyObject) -> *mut AnyObject =
        std::mem::transmute::<*mut c_void, _>(proxy_fn);
    let failover_fn: unsafe extern "C" fn(*mut AnyObject, bool) =
        std::mem::transmute::<*mut c_void, _>(failover_fn);
    let port = CString::new(address.port().to_string()).map_err(|_| POLICY_ERROR)?;
    let endpoint = Retained::from_raw(endpoint_fn(c"127.0.0.1".as_ptr(), port.as_ptr()))
        .ok_or(POLICY_ERROR)?;
    let proxy =
        Retained::from_raw(proxy_fn(Retained::as_ptr(&endpoint).cast_mut())).ok_or(POLICY_ERROR)?;
    failover_fn(Retained::as_ptr(&proxy).cast_mut(), false);
    let proxies = NSArray::from_retained_slice(&[proxy]);
    let _: () = msg_send![&store, setProxyConfigurations: &*proxies];
    let installed: Retained<NSArray<AnyObject>> = msg_send![&store, proxyConfigurations];
    let equal: Bool = msg_send![&installed, isEqualToArray: &*proxies];
    if !equal.as_bool() {
        return Err(POLICY_ERROR.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    #[test]
    fn production_rules_compile_in_native_webkit() {
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../scripts/compile-browser-rules.swift");
        let result = crate::process_runner::capture(
            std::process::Command::new("swift")
                .arg(script)
                .arg(local_resource_rules()),
            // Cold Swift/WebKit SDK module compilation can exceed 40 seconds;
            // the script separately bounds rule compilation itself to 15s.
            std::time::Duration::from_secs(120),
        )
        .expect("macOS builds require Xcode command-line tools");
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    #[test]
    fn native_blocklist_covers_all_resource_types_without_exception_rules() {
        let rules: serde_json::Value = serde_json::from_str(&local_resource_rules()).unwrap();
        assert_eq!(rules.as_array().unwrap().len(), 13);
        for rule in rules.as_array().unwrap() {
            assert_eq!(rule["action"]["type"], "block");
            assert!(rule["trigger"]["resource-type"].is_null());
            assert!(rule["trigger"]["unless-domain"].is_null());
            assert!(!rule["trigger"]["url-filter"]
                .as_str()
                .unwrap()
                .contains('|'));
        }
    }
    #[test]
    fn windows_has_no_loopback_or_udp_bypass_and_keeps_browser_security() {
        let args = windows_arguments("127.0.0.1:12345".parse().unwrap());
        for required in [
            "--proxy-server=socks5://127.0.0.1:12345",
            "--proxy-bypass-list=<-loopback>",
            "--webrtc-ip-handling-policy=disable_non_proxied_udp",
            "--disable-quic",
            "--disable-blink-features=WebTransport,DirectSockets",
        ] {
            assert!(args.contains(required));
        }
        for forbidden in [
            "direct://",
            "ignore-certificate",
            "disable-web-security",
            "no-sandbox",
            "msSmartScreenProtection",
        ] {
            assert!(!args.contains(forbidden));
        }
    }
}
