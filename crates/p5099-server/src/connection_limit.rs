use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Arc, Mutex, PoisonError},
};

#[derive(Debug)]
pub(crate) struct ConnectionLimit {
    total: usize,
    per_address: usize,
    open: Mutex<HashMap<IpAddr, usize>>,
}

#[derive(Debug)]
pub(crate) struct ConnectionPermit {
    limit: Arc<ConnectionLimit>,
    address: IpAddr,
}

impl ConnectionLimit {
    pub(crate) fn new(total: usize, per_address: usize) -> Arc<Self> {
        Arc::new(Self {
            total,
            per_address,
            open: Mutex::new(HashMap::new()),
        })
    }

    pub(crate) fn try_admit(self: &Arc<Self>, address: IpAddr) -> Option<ConnectionPermit> {
        let address = address.to_canonical();
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        let total: usize = open.values().sum();
        let count = open.get(&address).copied().unwrap_or(0);
        if total >= self.total || count >= self.per_address {
            return None;
        }
        open.insert(address, count + 1);
        Some(ConnectionPermit {
            limit: Arc::clone(self),
            address,
        })
    }
}

impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        let mut open = self
            .limit
            .open
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(count) = open.get_mut(&self.address) {
            *count -= 1;
            if *count == 0 {
                open.remove(&self.address);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use super::ConnectionLimit;

    const A: IpAddr = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 1));
    const B: IpAddr = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 2));

    #[test]
    fn limits_each_address_and_the_total_and_frees_on_drop() {
        let limit = ConnectionLimit::new(3, 2);
        let first = limit.try_admit(A).unwrap();
        let _second = limit.try_admit(A).unwrap();
        assert!(limit.try_admit(A).is_none());
        let mapped = IpAddr::V6(Ipv4Addr::new(203, 0, 113, 1).to_ipv6_mapped());
        assert!(limit.try_admit(mapped).is_none());
        let _third = limit.try_admit(B).unwrap();
        assert!(limit.try_admit(B).is_none());
        assert!(limit.try_admit(IpAddr::V6(Ipv6Addr::LOCALHOST)).is_none());
        drop(first);
        assert!(limit.try_admit(A).is_some());
        assert!(limit.open.lock().unwrap().values().sum::<usize>() <= 3);
    }
}
