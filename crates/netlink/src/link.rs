//! Link operations over rtnetlink — the Rust equivalents of the `ip link`
//! shell-outs in `mininet/util.py` (`makeIntfPair`, `moveIntf`) and
//! `mininet/node.py`/`mininet/link.py` (`ip link set dev ... up`,
//! `ip link set ... name ...`, `ip link set ... address ...`).

use futures::StreamExt;
use netlink_packet_route::link::{LinkAttribute, LinkFlags, LinkInfo as LinkInfoAttr, LinkMessage};
use rtnetlink::{LinkUnspec, LinkVeth};

use crate::{Error, Netlink, Result};

/// Maximum length of an interface name (IFNAMSIZ - 1), like iproute2.
pub const IFNAMSIZ_MAX: usize = 15;

/// Summary of a kernel link (interface), from `RTM_GETLINK` dumps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkInfo {
    /// Kernel interface index.
    pub index: u32,
    /// Interface name (`lo`, `eth0`, `h1-eth0`, ...).
    pub name: String,
    /// Whether `IFF_UP` is set.
    pub is_up: bool,
    /// Kind string (`veth`, `device`, `bridge`, ...) if reported.
    pub kind: Option<String>,
}

impl LinkInfo {
    fn from_message(msg: &LinkMessage) -> Self {
        let name = msg
            .attributes
            .iter()
            .find_map(|a| match a {
                LinkAttribute::IfName(n) => Some(n.clone()),
                _ => None,
            })
            .unwrap_or_default();
        let kind = msg.attributes.iter().find_map(|a| match a {
            LinkAttribute::LinkInfo(infos) => infos.iter().find_map(|i| match i {
                LinkInfoAttr::Kind(k) => Some(k.to_string()),
                _ => None,
            }),
            _ => None,
        });
        Self {
            index: msg.header.index,
            name,
            is_up: msg.header.flags.contains(LinkFlags::Up),
            kind,
        }
    }
}

/// Validate an interface name (IFNAMSIZ rules: ≤ 15 bytes, no `/`, `:`,
/// NUL or whitespace).
pub fn validate_ifname(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > IFNAMSIZ_MAX {
        return Err(Error::InvalidInput(format!(
            "interface name must be 1..={IFNAMSIZ_MAX} bytes: {name:?}"
        )));
    }
    if name
        .bytes()
        .any(|b| matches!(b, b'/' | b':' | b'\0') || b.is_ascii_whitespace())
    {
        return Err(Error::InvalidInput(format!(
            "interface name contains invalid characters: {name:?}"
        )));
    }
    Ok(())
}

/// Parse a MAC address string (`00:11:22:33:44:55`) into 6 bytes.
pub fn parse_mac(s: &str) -> Result<[u8; 6]> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 6 {
        return Err(Error::InvalidInput(format!(
            "MAC address must have 6 octets: {s:?}"
        )));
    }
    let mut mac = [0u8; 6];
    for (i, p) in parts.iter().enumerate() {
        if p.len() != 2 {
            return Err(Error::InvalidInput(format!(
                "MAC octet must be 2 hex digits: {s:?}"
            )));
        }
        mac[i] = u8::from_str_radix(p, 16)
            .map_err(|_| Error::InvalidInput(format!("bad MAC hex: {s:?}")))?;
    }
    Ok(mac)
}

impl Netlink {
    /// Dump all links (equivalent of `ip link show`). Works unprivileged.
    pub fn link_list(&self) -> Result<Vec<LinkInfo>> {
        self.run(async {
            let mut stream = self.handle.link().get().execute();
            let mut out = Vec::new();
            while let Some(msg) = stream.next().await {
                out.push(LinkInfo::from_message(&msg?));
            }
            Ok(out)
        })
    }

    /// Look up the kernel index of interface `name` (`ip link show dev <name>`).
    ///
    /// Note: a name-filtered `RTM_GETLINK` makes the kernel return an error
    /// when the device does not exist, so we dump and filter in userspace
    /// instead — `Ok(None)` means "no such interface".
    pub fn link_index(&self, name: &str) -> Result<Option<u32>> {
        Ok(self
            .link_list()?
            .into_iter()
            .find(|l| l.name == name)
            .map(|l| l.index))
    }

    /// Full [`LinkInfo`] for `name`, or `None` if it does not exist.
    pub fn link_get(&self, name: &str) -> Result<Option<LinkInfo>> {
        Ok(self.link_list()?.into_iter().find(|l| l.name == name))
    }

    fn require_index(&self, name: &str) -> Result<u32> {
        self.link_index(name)?
            .ok_or_else(|| Error::Other(format!("no such interface: {name:?}")))
    }

    /// Create a veth pair (equivalent of
    /// `ip link add name <name> type veth peer name <peer>`).
    ///
    /// Fails if either name already exists. Optionally set MAC addresses on
    /// both ends, like `makeIntfPair(..., addr1, addr2)` in
    /// `mininet/util.py`.
    pub fn veth_create(
        &self,
        name: &str,
        peer: &str,
        mac: Option<[u8; 6]>,
        peer_mac: Option<[u8; 6]>,
    ) -> Result<()> {
        validate_ifname(name)?;
        validate_ifname(peer)?;
        if name == peer {
            return Err(Error::InvalidInput(
                "veth pair ends must have different names".to_string(),
            ));
        }
        self.run(async {
            let msg = LinkVeth::new(name, peer).build();
            self.handle.link().add(msg).execute().await?;
            Ok::<(), rtnetlink::Error>(())
        })?;
        if let Some(mac) = mac {
            self.link_set_mac(name, mac)?;
        }
        if let Some(peer_mac) = peer_mac {
            self.link_set_mac(peer, peer_mac)?;
        }
        Ok(())
    }

    /// Create a veth pair, deleting any pre-existing interfaces with the
    /// same names first — mirrors `makeIntfPair(..., deleteIntfs=True)` which
    /// runs `ip link del <intf>` before creating the pair.
    pub fn veth_create_replacing(
        &self,
        name: &str,
        peer: &str,
        mac: Option<[u8; 6]>,
        peer_mac: Option<[u8; 6]>,
    ) -> Result<()> {
        for intf in [name, peer] {
            if self.link_index(intf)?.is_some() {
                self.link_delete(intf)?;
            }
        }
        self.veth_create(name, peer, mac, peer_mac)
    }

    /// Delete an interface (equivalent of `ip link del <name>`).
    pub fn link_delete(&self, name: &str) -> Result<()> {
        let idx = self.require_index(name)?;
        self.run(async { self.handle.link().del(idx).execute().await })?;
        Ok(())
    }

    /// Bring an interface up or down (equivalent of
    /// `ip link set dev <name> up|down`).
    pub fn link_set_up(&self, name: &str, up: bool) -> Result<()> {
        let idx = self.require_index(name)?;
        self.run(async {
            let builder = LinkUnspec::new_with_index(idx);
            let builder = if up { builder.up() } else { builder.down() };
            let msg = builder.build();
            self.handle.link().set(msg).execute().await
        })?;
        Ok(())
    }

    /// Rename an interface (equivalent of `ip link set <name> name <new>`).
    pub fn link_rename(&self, name: &str, new_name: &str) -> Result<()> {
        validate_ifname(new_name)?;
        let idx = self.require_index(name)?;
        self.run(async {
            let msg = LinkUnspec::new_with_index(idx).name(new_name).build();
            self.handle.link().set(msg).execute().await
        })?;
        Ok(())
    }

    /// Set the MAC address of an interface (equivalent of
    /// `ip link set dev <name> address <mac>`).
    pub fn link_set_mac(&self, name: &str, mac: [u8; 6]) -> Result<()> {
        let idx = self.require_index(name)?;
        self.run(async {
            let msg = LinkUnspec::new_with_index(idx)
                .address(mac.to_vec())
                .build();
            self.handle.link().set(msg).execute().await
        })?;
        Ok(())
    }

    /// Move an interface into the network namespace of process `pid`
    /// (equivalent of `ip link set <name> netns <pid>`, i.e.
    /// `moveIntf(intf, node)` in `mininet/util.py`).
    pub fn move_to_netns_pid(&self, name: &str, pid: u32) -> Result<()> {
        let idx = self.require_index(name)?;
        self.run(async {
            let msg = LinkUnspec::new_with_index(idx).setns_by_pid(pid).build();
            self.handle.link().set(msg).execute().await
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root_tests_enabled;

    #[test]
    fn parse_mac_accepts_valid_addresses() {
        assert_eq!(
            parse_mac("00:11:22:33:44:55").unwrap(),
            [0x00, 0x11, 0x22, 0x33, 0x44, 0x55]
        );
        assert_eq!(
            parse_mac("aa:bb:cc:dd:ee:ff").unwrap(),
            [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]
        );
    }

    #[test]
    fn parse_mac_rejects_invalid_addresses() {
        for bad in [
            "",
            "00:11:22:33:44",
            "00:11:22:33:44:55:66",
            "0:11:22:33:44:55",
            "00:11:22:33:44:5g",
            "00-11-22-33-44-55",
        ] {
            assert!(parse_mac(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn validate_ifname_enforces_ifnamsiz() {
        assert!(validate_ifname("h1-eth0").is_ok());
        assert!(validate_ifname("a").is_ok());
        assert!(validate_ifname(&"x".repeat(15)).is_ok());
        assert!(validate_ifname("").is_err());
        assert!(validate_ifname(&"x".repeat(16)).is_err());
        assert!(validate_ifname("a/b").is_err());
        assert!(validate_ifname("a:b").is_err());
        assert!(validate_ifname("a b").is_err());
    }

    // ------------------------------------------------------------------
    // Root-gated tests. These create/move/delete kernel interfaces and
    // must run on a privileged Linux host/CI runner:
    //   MN_RUST_ROOT_TESTS=1 cargo test -p mn-netlink -- --ignored --test-threads=1
    // ------------------------------------------------------------------

    #[test]
    #[ignore = "requires root + netns support on a privileged Linux host/CI; run with MN_RUST_ROOT_TESTS=1 cargo test -p mn-netlink -- --ignored --test-threads=1"]
    fn veth_pair_lifecycle() {
        if !root_tests_enabled() {
            return;
        }
        let nl = Netlink::new().unwrap();
        let name = format!("mveth{}a", std::process::id() % 100000);
        let peer = format!("mveth{}b", std::process::id() % 100000);
        let mac = parse_mac("00:00:00:00:00:42").unwrap();

        nl.veth_create_replacing(&name, &peer, Some(mac), None)
            .expect("veth_create as root");
        let a = nl.link_get(&name).unwrap().expect("veth end exists");
        let b = nl.link_get(&peer).unwrap().expect("peer end exists");
        assert_eq!(a.kind.as_deref(), Some("veth"));
        assert_ne!(a.index, b.index);

        nl.link_set_up(&name, true).unwrap();
        assert!(nl.link_get(&name).unwrap().unwrap().is_up);
        nl.link_set_up(&name, false).unwrap();
        assert!(!nl.link_get(&name).unwrap().unwrap().is_up);

        nl.link_rename(&peer, &format!("{peer}r")).unwrap();
        assert!(nl.link_index(&peer).unwrap().is_none());
        assert!(nl.link_index(&format!("{peer}r")).unwrap().is_some());

        nl.link_delete(&name).unwrap();
        nl.link_delete(&format!("{peer}r")).unwrap();
        assert!(nl.link_index(&name).unwrap().is_none());
    }

    #[test]
    #[ignore = "requires root + netns support on a privileged Linux host/CI; run with MN_RUST_ROOT_TESTS=1 cargo test -p mn-netlink -- --ignored --test-threads=1"]
    fn move_intf_to_netns_by_pid() {
        if !root_tests_enabled() {
            return;
        }
        use crate::ns::{netns_add, netns_del, setns_name};
        let ns = format!("mnrt{}", std::process::id());
        let _ = netns_del(&ns);
        netns_add(&ns).unwrap();

        // Spawn a child that lives in the new netns so we have a pid to
        // move interfaces to (like a mininet host process).
        // SAFETY: child calls setns_name + pause loop + raw _exit only.
        let holder = unsafe { nix::unistd::fork().unwrap() };
        let holder_pid = match holder {
            nix::unistd::ForkResult::Child => {
                if setns_name(&ns).is_err() {
                    // SAFETY: raw syscall.
                    unsafe { libc::_exit(1) };
                }
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(60));
                }
            }
            nix::unistd::ForkResult::Parent { child } => child,
        };

        let nl = Netlink::new().unwrap();
        let name = format!("mveth{}a", std::process::id() % 100000);
        let peer = format!("mveth{}b", std::process::id() % 100000);
        nl.veth_create(&name, &peer, None, None).unwrap();

        // moveIntf equivalent: move peer into the holder's netns.
        nl.move_to_netns_pid(&peer, holder_pid.as_raw() as u32)
            .expect("move to netns by pid");
        assert!(
            nl.link_index(&peer).unwrap().is_none(),
            "peer must be gone from the current ns after the move"
        );

        // Move it back using our own pid (we are in the original ns).
        nl.move_to_netns_pid(&peer, std::process::id()).unwrap();
        assert!(nl.link_index(&peer).unwrap().is_some());

        nl.link_delete(&name).unwrap();
        nl.link_delete(&peer).unwrap();
        // SAFETY: kill the holder child.
        unsafe { libc::kill(holder_pid.as_raw(), libc::SIGKILL) };
        nix::sys::wait::waitpid(holder_pid, None).ok();
        netns_del(&ns).unwrap();
    }
}
