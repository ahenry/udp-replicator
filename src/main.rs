extern crate pnet;

use pnet::packet::{MutablePacket, Packet};
use pnet::packet::udp::{ipv4_checksum, UdpPacket, MutableUdpPacket};
use pnet::packet::ipv4;
use pnet::packet::ipv4::{Ipv4Packet, MutableIpv4Packet};
#[allow(unused_imports)] use pnet::packet::ipv6;
#[allow(unused_imports)] use pnet::packet::ipv6::{Ipv6Packet, MutableIpv6Packet};
use pnet::packet::ip::IpNextHeaderProtocols;
use pnet::transport::{transport_channel, ipv4_packet_iter};
use pnet::transport::TransportChannelType::{Layer3};

use std::collections::HashMap;
#[allow(unused_imports)] use std::net::{IpAddr, SocketAddr};
use std::net::{Ipv4Addr, SocketAddrV4};
#[allow(unused_imports)] use std::net::{Ipv6Addr, SocketAddrV6};

enum LoadBalancingStrategy {
    Duplicate,
    RoundRobin,
}

enum Destination {
    Address(SocketAddr),
    Group(LoadBalanceGroup)
}

struct LoadBalanceGroup {
    strategy: LoadBalancingStrategy,
    destinations: Vec<Destination>
}

fn v4_to_v4<'a>(packet: &'a [u8], destination: &SocketAddrV4, source: Option<&Ipv4Addr>) -> Option<MutableIpv4Packet<'a>> {

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

type DestMap = HashMap<u16, Vec<SocketAddrV4>>;
enum IpPacket<'p> {
    V4(&'p Ipv4Packet<'p>),
    V6(&'p Ipv6Packet<'p>),
}

// TODO: this needs to return multiple addresses, etc
fn get_destinations<'a>(packet: IpPacket, dest_map: &'a DestMap) -> Option<&'a Vec<SocketAddrV4>> {
    let dest = match packet {
        IpPacket::V4(ref p) => {
            let udp_packet = UdpPacket::new(p.payload())?;
            udp_packet.get_destination()
        }
        IpPacket::V6(_) =>  unimplemented!("haven't done v6 support yet")
    };

    dest_map.get(&dest)
}

fn main() {

    // pretend we have parsed a config file, and generated this structure
    //  mapping local ports to the destination address
    //  TODO:
    //  * the destination needs to expand to handle load-balance and replicate
    //    workloads
    //  * the source may need to expand to be able to consider the local address
    //    as well as the port, but it's simpler for now just to use the port
    let dest_map: DestMap = [
        (333, vec!["127.1.0.1:3333".parse().unwrap(), "127.1.100.88:3333".parse().unwrap()]),
        (33, vec!["127.1.1.1:444".parse().unwrap()]),
    ].iter().cloned().collect();

    // TODO: probably need to replicate this whole thing for ipv6 too
    let protocol = Layer3(IpNextHeaderProtocols::Udp);

    let (mut tx, mut rx) = match transport_channel(4096, protocol) {
        Ok((tx, rx)) => (tx, rx),
        Err(e) => {
            panic!(
                "An error occurred when creating the transport channel: {}",
                e
            )
        }
    };

    let mut iter = ipv4_packet_iter(&mut rx);
    loop {
        if let Ok((packet, _addr)) = iter.next() {
//            println!("{:?}, {:?}", packet, _addr);
            
            let destinations = get_destinations(IpPacket::V4(&packet), &dest_map);
//            println!("{:?}", destinations);
            // if we didn't find a destination, we don't handle this packet
            if let None = destinations {
                continue;
            }

            let destinations = destinations.unwrap();

            for dest in destinations {
                let new_packet = v4_to_v4(packet.packet(), dest, None).unwrap();
                tx.send_to(new_packet, std::net::IpAddr::V4(*dest.ip())).unwrap();
            }
        }
    }
}
