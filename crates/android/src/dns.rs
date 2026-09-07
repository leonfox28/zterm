//! Safe Android DNS adaptation using the system's typed LinkProperties values.
use crate::NativeError;
use iroh::dns::{
    BoxIter, DNS_TIMEOUT, DnsError, DnsProtocol, DnsResolver, Resolver, TxtRecordData,
};
use std::{
    future::Future,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6},
    pin::Pin,
    sync::{Arc, RwLock},
};

/// One system DNS server, including IPv6 interface scope.
#[derive(Clone, Debug, uniffi::Record)]
pub struct NativeDnsServer {
    /// Four IPv4 or sixteen IPv6 network-order octets.
    pub address: Vec<u8>,
    /// Android's IPv6 scope ID; zero for unscoped/IPv4 addresses.
    pub scope_id: u32,
}
#[derive(Clone)]
pub(crate) struct AndroidDns {
    addresses: Arc<RwLock<Vec<SocketAddr>>>,
    resolver: DnsResolver,
}
impl AndroidDns {
    pub(crate) fn new(servers: Vec<NativeDnsServer>) -> Result<Self, NativeError> {
        let addresses = Arc::new(RwLock::new(decode(servers)?));
        let resolver = DnsResolver::custom(SystemDns::new(Arc::clone(&addresses)));
        Ok(Self {
            addresses,
            resolver,
        })
    }
    pub(crate) fn update(&self, servers: Vec<NativeDnsServer>) -> Result<(), NativeError> {
        let addresses = decode(servers)?;
        let mut current = self
            .addresses
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *current != addresses {
            *current = addresses;
            drop(current);
            self.resolver.reset();
        }
        Ok(())
    }
    pub(crate) fn resolver(&self) -> DnsResolver {
        self.resolver.clone()
    }
}
fn decode(servers: Vec<NativeDnsServer>) -> Result<Vec<SocketAddr>, NativeError> {
    if servers.len() > 8 {
        return Err(NativeError::RequestFailed {
            code: "resource_exhausted".to_owned(),
        });
    }
    servers
        .into_iter()
        .map(|server| match server.address.as_slice() {
            [a, b, c, d] => Ok(SocketAddr::new(Ipv4Addr::new(*a, *b, *c, *d).into(), 53)),
            bytes if bytes.len() == 16 => {
                let bytes: [u8; 16] = bytes
                    .try_into()
                    .map_err(|_| NativeError::RuntimeUnavailable)?;
                Ok(SocketAddr::V6(SocketAddrV6::new(
                    Ipv6Addr::from(bytes),
                    53,
                    0,
                    server.scope_id,
                )))
            }
            _ => Err(NativeError::RequestFailed {
                code: "address_unavailable".to_owned(),
            }),
        })
        .collect()
}
struct SystemDns {
    addresses: Arc<RwLock<Vec<SocketAddr>>>,
    current: DnsResolver,
}
impl std::fmt::Debug for SystemDns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AndroidSystemDns([REDACTED])")
    }
}
impl SystemDns {
    fn new(addresses: Arc<RwLock<Vec<SocketAddr>>>) -> Self {
        let values = addresses
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        // No with_system_defaults: Kotlin has already read the active network safely.
        let current = DnsResolver::builder()
            .with_nameservers(values.into_iter().map(|value| (value, DnsProtocol::Udp)))
            .build();
        Self { addresses, current }
    }
}
type DnsFuture<T> = Pin<Box<dyn Future<Output = Result<BoxIter<T>, DnsError>> + Send + 'static>>;
impl Resolver for SystemDns {
    fn lookup_ipv4(&self, host: String) -> DnsFuture<Ipv4Addr> {
        let dns = self.current.clone();
        Box::pin(async move {
            let values = dns
                .lookup_ipv4(host, DNS_TIMEOUT)
                .await?
                .filter_map(|ip| match ip {
                    IpAddr::V4(ip) => Some(ip),
                    _ => None,
                })
                .collect::<Vec<_>>();
            Ok(Box::new(values.into_iter()) as BoxIter<Ipv4Addr>)
        })
    }
    fn lookup_ipv6(&self, host: String) -> DnsFuture<Ipv6Addr> {
        let dns = self.current.clone();
        Box::pin(async move {
            let values = dns
                .lookup_ipv6(host, DNS_TIMEOUT)
                .await?
                .filter_map(|ip| match ip {
                    IpAddr::V6(ip) => Some(ip),
                    _ => None,
                })
                .collect::<Vec<_>>();
            Ok(Box::new(values.into_iter()) as BoxIter<Ipv6Addr>)
        })
    }
    fn lookup_txt(&self, host: String) -> DnsFuture<TxtRecordData> {
        let dns = self.current.clone();
        Box::pin(async move {
            let values = dns.lookup_txt(host, DNS_TIMEOUT).await?.collect::<Vec<_>>();
            Ok(Box::new(values.into_iter()) as BoxIter<TxtRecordData>)
        })
    }
    fn clear_cache(&self) {
        self.current.clear_cache();
    }
    fn reset(&self) -> Box<dyn Resolver> {
        Box::new(Self::new(Arc::clone(&self.addresses)))
    }
}
