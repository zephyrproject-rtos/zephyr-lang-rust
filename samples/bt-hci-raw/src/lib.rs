// Copyright (c) 2026
// SPDX-License-Identifier: Apache-2.0

#![no_std]

use bt_hci::cmd::info::{ReadLocalVersionInformation, ReadLocalVersionInformationReturn};
use bt_hci::cmd::Cmd;
use bt_hci::event::{CommandComplete, CommandCompleteWithStatus, EventKind};
use bt_hci::param::Status;
use bt_hci::transport::blocking::Transport;
use bt_hci::{ControllerToHostPacket, FromHciBytes};
use log::info;
use zephyr::bluetooth::hci_raw::{HciRawTransport, MAX_PACKET_SIZE};

#[no_mangle]
extern "C" fn rust_main() {
    // SAFETY: Zephyr initializes the UART console before invoking Rust.
    if unsafe { zephyr::set_logger() }.is_err() {
        zephyr::printkln!("Failed to install logger");
        return;
    }

    let transport = match HciRawTransport::enable() {
        Ok(transport) => transport,
        Err(error) => {
            zephyr::printkln!("Failed to enable HCI RAW: {error:?}");
            return;
        }
    };

    let command = ReadLocalVersionInformation::new();
    if let Err(error) = transport.write(&command) {
        zephyr::printkln!("Read Local Version Information write failed: {error:?}");
        return;
    }

    let mut rx = [0u8; MAX_PACKET_SIZE];
    let packet = match transport.read(&mut rx) {
        Ok(packet) => packet,
        Err(error) => {
            zephyr::printkln!("HCI response read failed: {error:?}");
            return;
        }
    };
    let ControllerToHostPacket::Event(event) = packet else {
        zephyr::printkln!("Received a non-event HCI packet");
        return;
    };
    if event.kind != EventKind::CommandComplete {
        zephyr::printkln!("Expected Command Complete, received {:?}", event.kind);
        return;
    }

    let complete = match CommandComplete::from_hci_bytes_complete(event.data)
        .and_then(CommandCompleteWithStatus::try_from)
    {
        Ok(complete) => complete,
        Err(error) => {
            zephyr::printkln!("Command Complete decode failed: {error:?}");
            return;
        }
    };
    if complete.cmd_opcode != ReadLocalVersionInformation::OPCODE
        || complete.status != Status::SUCCESS
    {
        zephyr::printkln!("Read Local Version Information was not successful");
        return;
    }

    let version = match ReadLocalVersionInformationReturn::from_hci_bytes_complete(
        complete.return_param_bytes.as_ref(),
    ) {
        Ok(version) => version,
        Err(error) => {
            zephyr::printkln!("Version response decode failed: {error:?}");
            return;
        }
    };

    // SAFETY: HCI response structs are packed; their multi-byte fields may be unaligned.
    let company_identifier =
        unsafe { core::ptr::addr_of!(version.company_identifier).read_unaligned() };
    info!("Read Local Version Information completed");
    zephyr::printkln!(
        "Read Local Version Information succeeded; manufacturer {company_identifier:#06x}"
    );
}
