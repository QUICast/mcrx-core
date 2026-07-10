#![cfg(all(target_os = "linux", feature = "raw-shared-capture"))]

use mcrx_core::{RawSubscriptionConfig, SharedRawContext};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::error::Error;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

#[test]
#[ignore = "requires CAP_NET_RAW and a Linux network namespace with writable multicast sysctls"]
fn many_asm_and_ssm_memberships_share_one_capture_socket() -> Result<(), Box<dyn Error>> {
    assert!(
        Command::new("ip")
            .args(["link", "set", "lo", "up"])
            .status()?
            .success()
    );
    assert!(
        Command::new("sysctl")
            .args(["-w", "net.ipv4.igmp_max_memberships=2048"])
            .status()?
            .success()
    );

    let interface = Ipv4Addr::LOCALHOST;
    let mut context = SharedRawContext::new();
    let mut joined_ids = Vec::with_capacity(1_000);
    let mut first_asm = None;

    for index in 0..499u16 {
        let group = Ipv4Addr::new(239, 192, (index >> 8) as u8, index as u8);
        let mut config = RawSubscriptionConfig::asm(group);
        config.interface = Some(interface.into());
        let id = context.add_subscription(config.clone())?;
        context.join_subscription(id)?;
        joined_ids.push(id);
        first_asm.get_or_insert((id, config));
    }
    for index in 0..500u16 {
        let group = Ipv4Addr::new(232, 192, (index >> 8) as u8, index as u8);
        let mut config = RawSubscriptionConfig::ssm(group, interface);
        config.interface = Some(interface.into());
        let id = context.add_subscription(config)?;
        context.join_subscription(id)?;
        joined_ids.push(id);
    }

    let (first_asm_id, first_asm_config) = first_asm.expect("at least one ASM membership");
    let duplicate_asm_id = context.add_subscription(first_asm_config)?;
    context.join_subscription(duplicate_asm_id)?;
    joined_ids.push(duplicate_asm_id);

    assert_eq!(context.subscription_count(), 1_000);
    assert_eq!(context.active_membership_count(), 1_000);
    assert_eq!(context.capture_socket_count(), 1);

    let sender = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    sender.set_multicast_if_v4(&interface)?;
    sender.set_multicast_loop_v4(true)?;
    sender.bind(&SockAddr::from(SocketAddrV4::new(interface, 0)))?;
    sender.send_to(
        b"shared raw capture",
        &SockAddr::from(SocketAddrV4::new(Ipv4Addr::new(239, 192, 0, 0), 5_000)),
    )?;

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let mut packets = Vec::new();
        context.try_recv_batch_into(&mut packets, 64)?;
        if let Some(packet) = packets.into_iter().next() {
            assert_eq!(packet.packet.subscription_id, first_asm_id);
            assert_eq!(
                packet.matching_subscription_ids(),
                &[first_asm_id, duplicate_asm_id]
            );
            break;
        }
        if Instant::now() >= deadline {
            panic!("shared capture did not receive the locally emitted multicast datagram");
        }
        thread::sleep(Duration::from_millis(1));
    }

    for _ in 0..16 {
        sender.send_to(
            b"shared raw capture batch",
            &SockAddr::from(SocketAddrV4::new(Ipv4Addr::new(239, 192, 0, 0), 5_000)),
        )?;
    }

    let mut packets = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(2);
    while packets.len() < 16 && Instant::now() < deadline {
        let remaining = 16 - packets.len();
        context.try_recv_batch_into(&mut packets, remaining)?;
        if packets.len() < 16 {
            thread::sleep(Duration::from_millis(10));
        }
    }
    assert_eq!(packets.len(), 16);
    assert!(
        packets.iter().all(|packet| {
            packet.matching_subscription_ids() == [first_asm_id, duplicate_asm_id]
        })
    );

    context.leave_subscription(first_asm_id)?;
    assert_eq!(context.active_membership_count(), 999);
    assert_eq!(context.capture_socket_count(), 1);

    sender.send_to(
        b"shared raw capture after leave",
        &SockAddr::from(SocketAddrV4::new(Ipv4Addr::new(239, 192, 0, 0), 5_000)),
    )?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let mut packets = Vec::new();
        context.try_recv_batch_into(&mut packets, 64)?;
        if let Some(packet) = packets.into_iter().next() {
            assert_eq!(packet.matching_subscription_ids(), &[duplicate_asm_id]);
            break;
        }
        if Instant::now() >= deadline {
            panic!("remaining duplicate membership did not receive after the first left");
        }
        thread::sleep(Duration::from_millis(1));
    }

    context.remove_subscription(first_asm_id)?;
    for id in joined_ids {
        if id != first_asm_id {
            context.remove_subscription(id)?;
        }
    }
    assert_eq!(context.subscription_count(), 0);
    assert_eq!(context.active_membership_count(), 0);
    assert_eq!(context.capture_socket_count(), 0);

    Ok(())
}
