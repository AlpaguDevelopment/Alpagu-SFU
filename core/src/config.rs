use std::net::{Ipv4Addr, SocketAddr};

pub struct Config {
    pub advertised: SocketAddr,
    pub port: u16,
}

impl Config {
    pub fn parse(args: impl Iterator<Item = String>) -> Result<Option<Self>, String> {
        let mut ip = Ipv4Addr::LOCALHOST;
        let mut port = 7880;
        let mut udp_port = 7882;
        let mut args = args.peekable();
        while let Some(arg) = args.next() {
            if arg == "--version" {
                println!("{}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            if arg == "--help" {
                println!("sfu-core [--ip <LAN IPv4>] [--port 7880] [--udp-port 7882] [--version]");
                return Ok(None);
            }
            let value = args
                .next()
                .ok_or_else(|| format!("Missing value for {arg}"))?;
            match arg.as_str() {
                "--ip" => ip = value.parse().map_err(|_| "Invalid IPv4 address")?,
                "--port" => port = value.parse().map_err(|_| "Invalid signaling port")?,
                "--udp-port" => udp_port = value.parse().map_err(|_| "Invalid UDP port")?,
                _ => return Err(format!("Unknown argument: {arg}")),
            }
        }
        if ip.is_unspecified()
            || ip.is_multicast()
            || ip.is_broadcast()
            || port == 0
            || udp_port == 0
        {
            return Err("Use a unicast IPv4 address and nonzero ports".into());
        }
        Ok(Some(Self {
            advertised: (ip, udp_port).into(),
            port,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_flags() {
        let c = Config::parse(
            ["--ip", "192.0.2.1", "--udp-port", "9000"]
                .into_iter()
                .map(str::to_owned),
        )
        .unwrap()
        .unwrap();
        assert_eq!(c.advertised, "192.0.2.1:9000".parse().unwrap());
        for args in [
            vec!["--port", "0"],
            vec!["--ip", "0.0.0.0"],
            vec!["--wat", "1"],
            vec!["--port"],
        ] {
            assert!(Config::parse(args.into_iter().map(str::to_owned)).is_err());
        }
    }
}
