use std::io::Read;
use std::os::unix::fs::FileExt;
use std::process::ExitCode;

const PCI_CAPABILITY_LIST: usize = 0x34;
const PCI_CAP_ID_EXP: u8 = 0x10;
const PCI_CAP_ID_EXP_LEN: usize = 0x3c;
const PCI_EXP_LNKCTL: usize = 0x10;
const PCI_EXP_LNKCTL_ASPM_L0S: u16 = 0x0001;
const PCI_EXP_LNKCTL_ASPM_L1: u16 = 0x0002;

fn find_pci_capability(
    config_buffer: &[u8],
    target_capability_id: u8,
    target_capability_length: usize,
) -> Option<std::ops::Range<usize>> {
    let mut capability_pointer = *config_buffer.get(PCI_CAPABILITY_LIST)? as usize;

    loop {
        let capability_id = *config_buffer.get(capability_pointer)?;
        let next_capability_pointer = *config_buffer.get(capability_pointer + 1)? as usize;

        if next_capability_pointer != 0 && next_capability_pointer < capability_pointer + 2 {
            eprintln!("error: next capability pointer invalid");
            return None;
        }

        if capability_id == target_capability_id {
            if (next_capability_pointer >= capability_pointer
                && target_capability_length > next_capability_pointer - capability_pointer)
                || (target_capability_length > config_buffer.len() - capability_pointer)
            {
                eprintln!("error: capability length overflow");
                return None;
            }

            return Some((capability_pointer)..(capability_pointer + target_capability_length));
        }

        if next_capability_pointer > capability_pointer {
            capability_pointer = next_capability_pointer;
        } else {
            return None;
        }
    }
}

fn find_pci_exp_link_control(config_buffer: &[u8]) -> Option<std::ops::Range<usize>> {
    let Some(capability_range) = find_pci_capability(&config_buffer, PCI_CAP_ID_EXP, PCI_CAP_ID_EXP_LEN) else {
        eprintln!("error: unable to find pci express capability structure");
        return None;
    };

    Some((capability_range.start + PCI_EXP_LNKCTL)..(capability_range.start + PCI_EXP_LNKCTL + 2))
}

#[derive(Debug)]
struct Args {
    mask: u16,
    flags: u16,
    path: String,
}

fn parse_args() -> Option<Args> {
    let mut path: Option<String> = None;
    let mut flags = 0;
    let mut mask = 0;

    let mut args = std::env::args();
    let _program = args.next();

    loop {
        let Some(arg) = args.next() else {
            break;
        };

        if let "--enable-l0s" = arg.as_str() {
            flags |= PCI_EXP_LNKCTL_ASPM_L0S;
            mask |= PCI_EXP_LNKCTL_ASPM_L0S;
        } else if let "--disable-l0s" = arg.as_str() {
            flags &= !PCI_EXP_LNKCTL_ASPM_L0S;
            mask |= PCI_EXP_LNKCTL_ASPM_L0S;
        } else if let "--enable-l1" = arg.as_str() {
            flags |= PCI_EXP_LNKCTL_ASPM_L1;
            mask |= PCI_EXP_LNKCTL_ASPM_L1;
        } else if let "--disable-l1" = arg.as_str() {
            flags &= !PCI_EXP_LNKCTL_ASPM_L1;
            mask |= PCI_EXP_LNKCTL_ASPM_L1;
        } else if arg.starts_with("--") {
            eprintln!("syntax: {}: unrecognized option", arg);
            return None;
        } else if let None = path {
            path = Some(arg);
        } else {
            eprintln!("syntax: {}: path already specified", arg);
            return None;
        }
    }

    let Some(path) = path else {
        eprintln!("syntax: missing path");
        return None;
    };

    return Some(Args {
        path: path,
        flags: flags,
        mask: mask,
    });
}

fn main() -> ExitCode {
    let Some(args) = parse_args() else {
        return ExitCode::from(1);
    };

    let mut config_file = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&args.path)
    {
        Ok(value) => value,
        Err(err) => {
            eprintln!("open: {}: {}", args.path, err);
            return ExitCode::from(1);
        }
    };

    let mut config_buffer = Vec::<u8>::new();

    if let Err(err) = config_file.read_to_end(&mut config_buffer) {
        eprintln!("read: {}: {}", args.path, err);
        return ExitCode::from(1);
    }

    let Some(link_control_range) = find_pci_exp_link_control(&config_buffer) else {
        return ExitCode::from(1);
    };

    let link_control_old_value = ((config_buffer[link_control_range.start + 1] as u16) << 8)
        | (config_buffer[link_control_range.start] as u16);
    let link_control_new_value = (link_control_old_value & !args.mask) | args.flags;

    if link_control_new_value != link_control_old_value {
        let link_control_new_buffer: [u8; 2] = [
            link_control_new_value as u8,
            (link_control_new_value >> 8) as u8,
        ];

        if let Err(err) =
            config_file.write_all_at(&link_control_new_buffer, link_control_range.start as u64)
        {
            eprintln!("write: {}: {}", args.path, err);
            return ExitCode::from(1);
        }
    }

    ExitCode::SUCCESS
}
