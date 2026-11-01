//! Route-based VPN check, used only for the private `.ru`/`.рф` decision.
use super::*;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VpnStatus {
    pub(crate) detected: bool,
    pub(crate) interface_name: Option<String>,
    pub(crate) confidence: String,
    pub(crate) detail: String,
}

pub(crate) fn is_tunnel_interface(interface_name: &str) -> bool {
    let value = interface_name.to_ascii_lowercase();
    ["utun", "tun", "tap", "wg", "wireguard", "ppp"]
        .iter()
        .any(|prefix| value.starts_with(prefix))
        || ["vpn", "tailscale", "mullvad", "nordlynx", "proton"]
            .iter()
            .any(|marker| value.contains(marker))
}

#[tauri::command]
pub(crate) async fn check_vpn(host: String) -> Result<VpnStatus, String> {
    if host.is_empty()
        || host.len() > 253
        || !host
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-'))
    {
        return Err("Некоректне доменне ім’я".into());
    }

    let ip = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::lookup_host((host.as_str(), 443)),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .and_then(|mut addresses| addresses.next())
    .map(|address| address.ip().to_string());
    tauri::async_runtime::spawn_blocking(move || check_vpn_route(ip))
        .await
        .map_err(|e| e.to_string())?
}

pub(crate) fn check_vpn_route(ip: Option<String>) -> Result<VpnStatus, String> {
    #[cfg(target_os = "macos")]
    {
        let interface_name = ip.as_ref().and_then(|ip| {
            process_runner::capture(
                Command::new("/sbin/route").args(["-n", "get", ip]),
                Duration::from_secs(5),
            )
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .find_map(|line| line.trim().strip_prefix("interface:").map(str::trim))
                    .map(str::to_string)
            })
        });
        let routed_through_tunnel = interface_name
            .as_deref()
            .map(is_tunnel_interface)
            .unwrap_or(false);
        let detected = routed_through_tunnel;
        let route_known = interface_name.is_some();
        let detail = if !route_known {
            "Не вдалося визначити маршрут до цього ресурсу"
        } else if routed_through_tunnel {
            "Маршрут саме до цього ресурсу проходить через VPN-тунель"
        } else {
            "Маршрут до цього ресурсу не проходить через VPN-тунель"
        };
        Ok(VpnStatus {
            detected,
            interface_name,
            confidence: if route_known { "high" } else { "unknown" }.into(),
            detail: detail.into(),
        })
    }

    #[cfg(target_os = "windows")]
    {
        let interface_name = ip.as_ref().and_then(|ip| {
            let mut command = Command::new("powershell.exe");
            configure_command(&mut command);
            command
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                ])
                .arg("-Command")
                .arg(windows_powershell_script(
                    "$route = Find-NetRoute -RemoteIPAddress $env:YTDLP_ROUTE_IP -ErrorAction Stop | Select-Object -First 1; $route.InterfaceAlias",
                ))
                .env("YTDLP_ROUTE_IP", ip);
            process_runner::capture(&mut command, Duration::from_secs(5))
                .ok()
                .filter(|output| output.status.success())
                .and_then(|output| {
                    String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .map(str::trim)
                        .find(|line| !line.is_empty())
                        .map(str::to_string)
                })
        });
        let routed_through_tunnel = interface_name
            .as_deref()
            .map(is_tunnel_interface)
            .unwrap_or(false);
        let route_known = interface_name.is_some();
        Ok(VpnStatus {
            detected: routed_through_tunnel,
            interface_name,
            confidence: if route_known { "high" } else { "unknown" }.into(),
            detail: if routed_through_tunnel {
                "Маршрут саме до цього ресурсу проходить через VPN-тунель"
            } else if route_known {
                "Маршрут до цього ресурсу не проходить через відомий VPN-інтерфейс"
            } else {
                "Не вдалося визначити Windows-інтерфейс для маршруту до цього ресурсу"
            }
            .into(),
        })
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Ok(VpnStatus {
        detected: false,
        interface_name: None,
        confidence: "unknown".into(),
        detail: "Перевірку VPN для цієї платформи ще не реалізовано".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_common_macos_and_windows_vpn_interfaces() {
        assert!(is_tunnel_interface("utun4"));
        assert!(is_tunnel_interface("WireGuard Tunnel"));
        assert!(is_tunnel_interface("NordLynx"));
        assert!(is_tunnel_interface("ProtonVPN"));
        assert!(!is_tunnel_interface("Ethernet"));
    }
}
