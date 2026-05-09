use std::net::IpAddr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReceiveCliArgs {
    pub(crate) group: IpAddr,
    pub(crate) dst_port: u16,
    pub(crate) source: Option<IpAddr>,
    pub(crate) interface: Option<IpAddr>,
}

pub(crate) fn parse_receive_cli_args(args: &[String]) -> Result<ReceiveCliArgs, String> {
    if args.len() < 3 {
        return Err("invalid arguments".to_string());
    }

    let group = parse_ip("group", &args[1])?;
    let dst_port = parse_port(&args[2])?;
    let remainder = &args[3..];

    let (source, interface) = parse_mixed_args(remainder)?;

    if !group.is_multicast() {
        return Err(format!("group address {group} is not multicast"));
    }

    Ok(ReceiveCliArgs {
        group,
        dst_port,
        source,
        interface,
    })
}

fn parse_flag_args(remainder: &[String]) -> Result<(Option<IpAddr>, Option<IpAddr>), String> {
    let mut source = None;
    let mut interface = None;
    let mut index = 0usize;

    while index < remainder.len() {
        match remainder[index].as_str() {
            "--source" => {
                let value = remainder
                    .get(index + 1)
                    .ok_or_else(|| "missing value after --source".to_string())?;
                source = Some(parse_ip("source", value)?);
                index += 2;
            }
            "--interface" => {
                let value = remainder
                    .get(index + 1)
                    .ok_or_else(|| "missing value after --interface".to_string())?;
                interface = Some(parse_ip("interface", value)?);
                index += 2;
            }
            other => {
                return Err(format!("unexpected argument '{other}'"));
            }
        }
    }

    Ok((source, interface))
}

fn parse_mixed_args(remainder: &[String]) -> Result<(Option<IpAddr>, Option<IpAddr>), String> {
    let mut positional = Vec::new();
    let mut flagged = Vec::new();
    let mut index = 0usize;

    while index < remainder.len() {
        if remainder[index].starts_with("--") {
            flagged.push(remainder[index].clone());
            let value = remainder
                .get(index + 1)
                .ok_or_else(|| format!("missing value after {}", remainder[index]))?;
            flagged.push(value.clone());
            index += 2;
        } else {
            positional.push(remainder[index].clone());
            index += 1;
        }
    }

    let (mut source, mut interface) = parse_flag_args(&flagged)?;

    let mut positional = positional.into_iter();

    if source.is_none()
        && let Some(value) = positional.next()
    {
        source = Some(parse_ip("source", &value)?);
    }

    if interface.is_none()
        && let Some(value) = positional.next()
    {
        interface = Some(parse_ip("interface", &value)?);
    }

    if positional.next().is_some() {
        return Err("invalid arguments".to_string());
    }

    Ok((source, interface))
}

fn parse_ip(name: &str, value: &str) -> Result<IpAddr, String> {
    value
        .parse::<IpAddr>()
        .map_err(|err| format!("invalid {name} '{value}': {err}"))
}

fn parse_port(value: &str) -> Result<u16, String> {
    let port = value
        .parse::<u16>()
        .map_err(|err| format!("invalid dst_port '{value}': {err}"))?;

    if port == 0 {
        return Err("dst_port must not be 0".to_string());
    }

    Ok(port)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| (*part).to_string()).collect()
    }

    #[test]
    fn parses_legacy_positional_asm_args() {
        let args = argv(&["mcrx-recv", "239.1.2.3", "5000"]);

        let parsed = parse_receive_cli_args(&args).unwrap();

        assert_eq!(parsed.group, IpAddr::V4(Ipv4Addr::new(239, 1, 2, 3)));
        assert_eq!(parsed.dst_port, 5000);
        assert_eq!(parsed.source, None);
        assert_eq!(parsed.interface, None);
    }

    #[test]
    fn parses_flagged_interface_for_ipv6_asm() {
        let args = argv(&["mcrx-recv-meta", "ff01::1234", "5000", "--interface", "::1"]);

        let parsed = parse_receive_cli_args(&args).unwrap();

        assert_eq!(
            parsed.group,
            IpAddr::V6("ff01::1234".parse::<Ipv6Addr>().unwrap())
        );
        assert_eq!(parsed.dst_port, 5000);
        assert_eq!(parsed.source, None);
        assert_eq!(parsed.interface, Some(IpAddr::V6(Ipv6Addr::LOCALHOST)));
    }

    #[test]
    fn parses_flagged_source_and_interface() {
        let args = argv(&[
            "mcrx-recv",
            "232.1.2.3",
            "5000",
            "--source",
            "192.168.1.10",
            "--interface",
            "192.168.1.20",
        ]);

        let parsed = parse_receive_cli_args(&args).unwrap();

        assert_eq!(
            parsed.source,
            Some(IpAddr::V4("192.168.1.10".parse::<Ipv4Addr>().unwrap()))
        );
        assert_eq!(
            parsed.interface,
            Some(IpAddr::V4("192.168.1.20".parse::<Ipv4Addr>().unwrap()))
        );
    }

    #[test]
    fn parses_positional_source_with_flagged_interface() {
        let args = argv(&[
            "mcrx-recv-meta",
            "ff12::1234",
            "5000",
            "fd06:ba51:f296:0:1caf:6b66:e6f7:4b10",
            "--interface",
            "fd06:ba51:f296:0:1caf:6b66:e6f7:4b10",
        ]);

        let parsed = parse_receive_cli_args(&args).unwrap();

        assert_eq!(
            parsed.source,
            Some(IpAddr::V6(
                "fd06:ba51:f296:0:1caf:6b66:e6f7:4b10"
                    .parse::<Ipv6Addr>()
                    .unwrap()
            ))
        );
        assert_eq!(
            parsed.interface,
            Some(IpAddr::V6(
                "fd06:ba51:f296:0:1caf:6b66:e6f7:4b10"
                    .parse::<Ipv6Addr>()
                    .unwrap()
            ))
        );
    }
}
