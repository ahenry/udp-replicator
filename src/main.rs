extern crate pnet;
#[macro_use]
extern crate serde_derive;
extern crate serde_yaml;

use pnet::packet::ip::IpNextHeaderProtocols;
use pnet::packet::ipv4;
use pnet::packet::ipv4::{Ipv4Packet, MutableIpv4Packet};
#[allow(unused_imports)]
use pnet::packet::ipv6;
#[allow(unused_imports)]
use pnet::packet::ipv6::{Ipv6Packet, MutableIpv6Packet};
use pnet::packet::udp::{ipv4_checksum, MutableUdpPacket, UdpPacket};
use pnet::packet::{MutablePacket, Packet};
use pnet::transport::TransportChannelType::Layer3;
use pnet::transport::{ipv4_packet_iter, transport_channel};

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::File;
#[allow(unused_imports)]
use std::net::{IpAddr, SocketAddr};
use std::net::{Ipv4Addr, SocketAddrV4};
#[allow(unused_imports)]
use std::net::{Ipv6Addr, SocketAddrV6};

trait Strategy {
    // XXX will probably need to find a way to make the return be a Vec<&SocketAddr> for speed,
    // but who knows, memcpy might be fast enough
    fn next_destinations(&self, destinations: &Vec<Destination>) -> Vec<SocketAddrV4>;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum LoadBalancingStrategy {
    Duplicate(Duplicate),
    RoundRobin(RoundRobin),
    WeightedRoundRobin,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Duplicate {}

impl Strategy for Duplicate {
    fn next_destinations(&self, destinations: &Vec<Destination>) -> Vec<SocketAddrV4> {
        let mut ret = Vec::new();

        for dest in destinations {
            match dest {
                &Destination::Address(sa) => ret.push(sa),
                &Destination::Group(ref group) => ret.extend(group.get_balance_result()),
            }
        }

        ret
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RoundRobin {
    #[serde(skip_serializing, default)]
    next_index: RefCell<usize>, // XXX thread-unsafety
}

// NOTE: thinking about dynamic health checks that might add or remove things from the pool.  It
// might be easier to just have the whole LoadBalanceGroup.strategy field be mutable for callbacks
// to have their way with
impl RoundRobin {
    #[allow(dead_code)]
    fn new() -> RoundRobin {
        RoundRobin {
            next_index: RefCell::new(0),
        }
    }
}

impl Strategy for RoundRobin {
    fn next_destinations(&self, destinations: &Vec<Destination>) -> Vec<SocketAddrV4> {
        let dest = &destinations[*self.next_index.borrow()]; // panic

        let mut ni = self.next_index.borrow_mut(); // panic
        *ni += 1;
        // len() should be cheap (basically a field lookup)
        if *ni >= destinations.len() {
            *ni = 0
        }

        let mut ret = Vec::new();
        match dest {
            &Destination::Address(sa) => ret.push(sa),
            &Destination::Group(ref group) => ret.extend(group.get_balance_result()),
        }

        ret
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum Destination {
    Address(SocketAddrV4),
    Group(LoadBalanceGroup),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LoadBalanceGroup {
    strategy: LoadBalancingStrategy,
    destinations: Vec<Destination>,
}

impl LoadBalanceGroup {
    fn get_balance_result(&self) -> Vec<SocketAddrV4> {
        match self.strategy {
            LoadBalancingStrategy::Duplicate(ref strategy) => {
                strategy.next_destinations(&self.destinations)
            }
            LoadBalancingStrategy::RoundRobin(ref strategy) => {
                strategy.next_destinations(&self.destinations)
            }
            _ => unimplemented!("haven't bothered with the others yet"),
        }
    }
}

fn v4_to_v4<'a>(
    packet: &'a [u8],
    destination: &SocketAddrV4,
    source: Option<&Ipv4Addr>,
) -> Option<MutableIpv4Packet<'a>> {
    let mut new_packet = MutableIpv4Packet::owned(packet.to_vec())?;

    if let Some(src_addr) = source {
        new_packet.set_source(*src_addr);
    }
    let new_source = new_packet.get_source();
    let new_dest = *destination.ip();
    new_packet.set_destination(new_dest);

    {
        let mut new_udp_packet = MutableUdpPacket::new(new_packet.payload_mut())?;
        new_udp_packet.set_destination(destination.port());
        let checksum = ipv4_checksum(&new_udp_packet.to_immutable(), &new_source, &new_dest);
        new_udp_packet.set_checksum(checksum);
    }

    let new_checksum = ipv4::checksum(&new_packet.to_immutable());
    new_packet.set_checksum(new_checksum);

    Some(new_packet)
}

type DestMap = HashMap<u16, LoadBalanceGroup>;
enum IpPacket<'p> {
    V4(&'p Ipv4Packet<'p>),
    V6(&'p Ipv6Packet<'p>),
}

// TODO: this needs to return multiple addresses, etc
fn get_destinations<'a>(packet: IpPacket, dest_map: &'a DestMap) -> Vec<SocketAddrV4> {
    let dest = match packet {
        IpPacket::V4(ref p) => match UdpPacket::new(p.payload()) {
            Some(udp_packet) => udp_packet.get_destination(),
            None => return Vec::new(),
        },
        IpPacket::V6(_) => unimplemented!("haven't done v6 support yet"),
    };

    let cfg = match dest_map.get(&dest) {
        Some(lbm) => lbm,
        None => return Vec::new(),
    };

    cfg.get_balance_result()
}

fn main() {
    // pretend we have parsed a config file, and generated this structure
    //  mapping local ports to the destination address
    //  TODO:
    //  * the source may need to expand to be able to consider the local address
    //    as well as the port, but it's simpler for now just to use the port
    let config = File::open("config.yaml").expect("Couldn't open config file");
    let dest_map: DestMap = serde_yaml::from_reader(config).expect("Couldn't parse config file");
    println!("{:?}", dest_map);

    // TODO: probably need to replicate this whole thing for ipv6 too
    let protocol = Layer3(IpNextHeaderProtocols::Udp);

    let (mut tx, mut rx) = match transport_channel(4096, protocol) {
        Ok((tx, rx)) => (tx, rx),
        Err(e) => panic!(
            "An error occurred when creating the transport channel: {}",
            e
        ),
    };

    let mut iter = ipv4_packet_iter(&mut rx);
    loop {
        if let Ok((packet, _addr)) = iter.next() {
            //            println!("{:?}, {:?}", packet, _addr);

            let destinations = get_destinations(IpPacket::V4(&packet), &dest_map);
            //            println!("{:?}", destinations);
            for dest in &destinations {
                let new_packet = v4_to_v4(packet.packet(), dest, None).unwrap();
                tx.send_to(new_packet, std::net::IpAddr::V4(*dest.ip()))
                    .unwrap();
            }
        }
    }
}

// vim: set fdm=marker:
