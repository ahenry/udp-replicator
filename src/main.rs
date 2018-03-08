extern crate pnet;

use pnet::packet::{MutablePacket, Packet};
use pnet::packet::udp::{ipv4_checksum, UdpPacket, MutableUdpPacket};
use pnet::packet::ipv4::{checksum, MutableIpv4Packet};
use pnet::packet::ip::IpNextHeaderProtocols;
use pnet::transport::{transport_channel, udp_packet_iter, ipv4_packet_iter};
use pnet::transport::TransportProtocol::Ipv4;
use pnet::transport::TransportChannelType::{Layer3, Layer4};

use std::iter::repeat;
use std::str;

fn main() {
    // udp only for the moment
    //let protocol = Layer4(Ipv4(IpNextHeaderProtocols::Udp));
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
        if let Ok((packet, addr)) = iter.next() {
            //println!("{:?}, {:?}", packet, addr);
            if packet.get_destination() == "127.0.0.3".parse::<std::net::Ipv4Addr>().unwrap() {
                continue;
            }

            if let Some(udp_packet) = UdpPacket::new(packet.payload()) {
                //println!("{:?}", udp_packet);
                if udp_packet.get_destination() == 333 {
                    println!("{:?}", str::from_utf8(udp_packet.payload()).unwrap());
                } else {
                    continue;
                }

                let old_source = packet.get_source();
                let old_dest = packet.get_destination();
                let new_dest: std::net::Ipv4Addr = "127.0.0.3".parse().unwrap();

                let mut backing: Vec<u8> = repeat(0u8).take(packet.packet().len()).collect();
                let mut new_packet = MutableIpv4Packet::new(&mut backing).unwrap();
                new_packet.clone_from(&packet);
                new_packet.set_destination(new_dest);

                {
                    let mut new_udp_packet = MutableUdpPacket::new(new_packet.payload_mut())
                        .unwrap();
                    let checksum =
                        ipv4_checksum(&new_udp_packet.to_immutable(), old_source, new_dest);
                    new_udp_packet.set_checksum(checksum);
                    //println!("{:?}", new_udp_packet);
                }

                let new_checksum = checksum(&new_packet.to_immutable());
                new_packet.set_checksum(new_checksum);

                //println!("{:?}", new_packet);
                let _ = tx.send_to(new_packet, "127.0.0.3".parse().unwrap());

            } else {
                continue;
            }
        }
    }
}
