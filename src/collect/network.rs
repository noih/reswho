use crate::model::{Network, Protocol, Socket};
use libproc::{
    file_info::PIDFDInfoFlavor,
    net_info::{InSIAddr, InSockInfo, SocketFDInfo, SocketInfoKind, TcpSIState},
};
use std::{
    collections::HashSet,
    io,
    mem::{MaybeUninit, size_of},
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6},
};

const MAX_DESCRIPTORS: usize = 65_536;
const INI_IPV4: u8 = 0x1;
const INI_IPV6: u8 = 0x2;

fn descriptors(pid: i32, expected: usize) -> io::Result<Vec<libc::proc_fdinfo>> {
    let mut capacity = expected.saturating_add(32).clamp(32, MAX_DESCRIPTORS);
    for _ in 0..3 {
        let mut fds = vec![
            libc::proc_fdinfo {
                proc_fd: 0,
                proc_fdtype: 0
            };
            capacity
        ];
        let bytes = unsafe {
            *libc::__error() = 0;
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDLISTFDS,
                0,
                fds.as_mut_ptr().cast(),
                (fds.len() * size_of::<libc::proc_fdinfo>()) as i32,
            )
        };
        let error = io::Error::last_os_error();
        if bytes < 0 || (bytes == 0 && error.raw_os_error() != Some(0)) {
            return Err(error);
        }
        let bytes = bytes as usize;
        if !bytes.is_multiple_of(size_of::<libc::proc_fdinfo>()) {
            return Err(io::Error::other("incomplete descriptor record"));
        }
        let count = bytes / size_of::<libc::proc_fdinfo>();
        if count < capacity {
            fds.truncate(count);
            return Ok(fds);
        }
        if capacity == MAX_DESCRIPTORS {
            break;
        }
        capacity = (capacity * 2).min(MAX_DESCRIPTORS);
    }
    Err(io::Error::other(
        "descriptor inventory changed too quickly or exceeded collection limit",
    ))
}

fn socket_info(pid: i32, fd: i32) -> io::Result<SocketFDInfo> {
    let mut value = MaybeUninit::<SocketFDInfo>::zeroed();
    let bytes = unsafe {
        libc::proc_pidfdinfo(
            pid,
            fd,
            PIDFDInfoFlavor::SocketInfo as i32,
            value.as_mut_ptr().cast(),
            size_of::<SocketFDInfo>() as i32,
        )
    };
    if bytes == size_of::<SocketFDInfo>() as i32 {
        Ok(unsafe { value.assume_init() })
    } else if bytes <= 0 {
        Err(io::Error::last_os_error())
    } else {
        Err(io::Error::other("incomplete socket record"))
    }
}

fn address(value: InSIAddr, port: i32, info: &InSockInfo, family: i32) -> io::Result<SocketAddr> {
    let port = u16::from_be(port as u16);
    if family == libc::AF_INET6 && info.insi_vflag & INI_IPV6 != 0 {
        let ip = Ipv6Addr::from(unsafe { value.ina_6.s6_addr });
        let scope = if ip.is_unicast_link_local() {
            u32::from(info.insi_v6.in6_ifindex)
        } else {
            0
        };
        Ok(SocketAddr::V6(SocketAddrV6::new(ip, port, 0, scope)))
    } else if info.insi_vflag & INI_IPV4 != 0 {
        let bytes = unsafe { value.ina_46.i46a_addr4.s_addr }.to_ne_bytes();
        Ok(SocketAddr::new(Ipv4Addr::from(bytes).into(), port))
    } else {
        Err(io::Error::other("socket IP version unavailable"))
    }
}

fn decode(fd: i32, record: &SocketFDInfo) -> io::Result<Option<Socket>> {
    let socket = &record.psi;
    if ![libc::AF_INET, libc::AF_INET6].contains(&socket.soi_family) {
        return Ok(None);
    }
    let (protocol, info, tcp_state) =
        match (SocketInfoKind::from(socket.soi_kind), socket.soi_protocol) {
            (SocketInfoKind::Tcp, libc::IPPROTO_TCP) => {
                let tcp = unsafe { socket.soi_proto.pri_tcp };
                let state = match TcpSIState::from(tcp.tcpsi_state) {
                    TcpSIState::Unknown => format!("Unknown({})", tcp.tcpsi_state),
                    state => format!("{state:?}"),
                };
                (Protocol::Tcp, tcp.tcpsi_ini, Some(state))
            }
            (SocketInfoKind::In, libc::IPPROTO_UDP) => {
                (Protocol::Udp, unsafe { socket.soi_proto.pri_in }, None)
            }
            (_, libc::IPPROTO_TCP | libc::IPPROTO_UDP) => {
                return Err(io::Error::other("unsupported TCP/UDP socket record"));
            }
            _ => return Ok(None),
        };
    Ok(Some(Socket {
        fd,
        protocol,
        local: address(info.insi_laddr, info.insi_lport, &info, socket.soi_family)?,
        remote: if info.insi_fport == 0 {
            None
        } else {
            Some(address(
                info.insi_faddr,
                info.insi_fport,
                &info,
                socket.soi_family,
            )?)
        },
        tcp_state,
    }))
}

pub(super) fn collect(pid: i32, expected_fds: usize) -> io::Result<Network> {
    let fds = descriptors(pid, expected_fds)?;
    let mut network = Network {
        complete: true,
        ..Network::default()
    };
    let mut seen = HashSet::new();
    for fd in fds
        .into_iter()
        .filter(|fd| fd.proc_fdtype == libc::PROX_FDTYPE_SOCKET as u32)
    {
        let result = socket_info(pid, fd.proc_fd).and_then(|record| {
            let socket = decode(fd.proc_fd, &record)?;
            if record.psi.soi_so == 0 || seen.insert(record.psi.soi_so) {
                Ok(socket)
            } else {
                Ok(None)
            }
        });
        match result {
            Ok(Some(socket)) => network.sockets.push(socket),
            Ok(None) => (),
            Err(error) => {
                network.complete = false;
                network
                    .unavailable
                    .push(format!("fd {}: {error}", fd.proc_fd));
            }
        }
    }
    network.sockets.sort_by_key(|socket| socket.fd);
    Ok(network)
}

#[cfg(test)]
mod tests {
    use super::*;
    use libproc::net_info::{SocketInfo, SocketInfoProto, TcpSockInfo};

    #[test]
    fn dual_stack_and_unrecognized_socket_metadata_do_not_become_ipv4_or_zero() {
        let mut record = SocketFDInfo {
            psi: SocketInfo {
                soi_family: libc::AF_INET6,
                soi_kind: SocketInfoKind::Tcp as i32,
                soi_protocol: libc::IPPROTO_TCP,
                soi_proto: SocketInfoProto {
                    pri_tcp: TcpSockInfo {
                        tcpsi_ini: InSockInfo {
                            insi_vflag: INI_IPV4 | INI_IPV6,
                            insi_lport: 8080u16.to_be() as i32,
                            insi_laddr: InSIAddr {
                                ina_6: libc::in6_addr { s6_addr: [0; 16] },
                            },
                            ..InSockInfo::default()
                        },
                        tcpsi_state: 99,
                        ..TcpSockInfo::default()
                    },
                },
                ..SocketInfo::default()
            },
            ..SocketFDInfo::default()
        };
        let decoded = decode(7, &record).unwrap().unwrap();
        assert_eq!(decoded.local.to_string(), "[::]:8080");
        assert_eq!(decoded.tcp_state.as_deref(), Some("Unknown(99)"));
        assert!(decoded.remote.is_none());
        record.psi.soi_proto.pri_tcp.tcpsi_ini.insi_vflag = 0;
        assert!(decode(7, &record).is_err());
        record.psi.soi_kind = 99;
        assert!(decode(7, &record).is_err());
        record.psi.soi_family = libc::AF_UNIX;
        assert!(decode(7, &record).unwrap().is_none());
    }
}
