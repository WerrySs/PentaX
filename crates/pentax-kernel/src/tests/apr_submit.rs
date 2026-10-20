// SPDX-License-Identifier: GPL-2.0-only
// Original PentaX fixtures; no console or title bytes.
use super::*;

const OBJECT: u64 = ARG + 0x500;
const BUFFER: u64 = ARG + 0x800;
const DESTINATION: u64 = ARG + 0x1800;
const RESULT: u64 = ARG + 0x2100;
const ID: u64 = ARG + 0x2120;
const STACK: u64 = ARG + 0x3000;

fn invoke(
    owner: &mut Kernel,
    guest: &mut TestGuest,
    service: Service,
    values: &[u64],
) -> Result<u64, Failure> {
    let mut args = [0; 6];
    args[..values.len()].copy_from_slice(values);
    owner.invoke_library(service, CallArgs(args), guest)
}
fn setup(limits: Limits) -> (OriginalFile, Kernel, TestGuest) {
    let file = OriginalFile::new();
    let mut owner = Kernel::new(
        limits,
        ProcessIdentity {
            process_id: 41,
            initial_thread_id: 73,
        },
        vec![file.root()],
    )
    .unwrap();
    let mut guest = TestGuest::new();
    guest.put(0x100, b"/original/data.txt\0");
    guest.put(0x200, &(ARG + 0x100).to_le_bytes());
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::AprResolveFilepathsToIdsAndFileSizes,
            &[ARG + 0x200, 1, ID, 0, 0]
        ),
        Ok(0)
    );
    assert_eq!(guest.get(ID, 4), 1_u32.to_le_bytes());
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::AmprSetBuffer,
            &[OBJECT, BUFFER, 128]
        ),
        Ok(0)
    );
    (file, owner, guest)
}
fn append(
    owner: &mut Kernel,
    guest: &mut TestGuest,
    id: u64,
    destination: u64,
    size: u64,
    offset: u64,
) {
    guest.put(STACK + 8 - ARG, &offset.to_le_bytes());
    assert_eq!(
        owner.invoke_library_with_stack(
            Service::AmprReadFile,
            CallArgs([OBJECT, 0, 0, id, destination, size]),
            Some(STACK),
            guest
        ),
        Ok(0)
    );
}
fn submit(owner: &mut Kernel, guest: &mut TestGuest) -> Result<u64, Failure> {
    invoke(
        owner,
        guest,
        Service::AprSubmitCommandBufferAndGetResult,
        &[OBJECT, u64::MAX, RESULT, ID],
    )
}
fn completion(guest: &TestGuest, error: i32, offset: u32) {
    let expected = [error.to_le_bytes(), offset.to_le_bytes()].concat();
    assert_eq!(guest.get(RESULT, 8), expected);
}
fn wait(owner: &mut Kernel, guest: &mut TestGuest, id: u64) -> Result<u64, Failure> {
    invoke(owner, guest, Service::AprWaitCommandBuffer, &[id])
}

#[test]
fn apr_submit_reads_current_file_chunks_repeats_without_consuming_queue_and_preserves_eof_tail() {
    let limits = Limits {
        max_io_bytes: 8,
        max_scratch_bytes: 128,
        max_name_bytes: 64,
        ..Default::default()
    };
    let (file, mut owner, mut guest) = setup(limits);
    append(&mut owner, &mut guest, 1, DESTINATION, 32, 0);
    guest.put(DESTINATION - ARG, &[0xa5; 40]);
    let errno = invoke(&mut owner, &mut guest, Service::ErrnoLocation, &[]).unwrap();
    guest.put(errno - ARG, &37_u32.to_le_bytes());
    assert_eq!(submit(&mut owner, &mut guest), Ok(0));
    assert_eq!(guest.get(DESTINATION, 24), b"original-data-0123456789");
    assert_eq!(guest.get(DESTINATION + 24, 16), [0xa5; 16]);
    completion(&guest, 0, 0);
    assert_eq!(guest.get(ID, 4), 1_u32.to_le_bytes());
    assert_eq!(guest.get(errno, 4), 37_u32.to_le_bytes());
    assert_eq!(owner.usage().ampr_commands, 1);
    assert_eq!(owner.usage().apr_submissions, 1);
    // Current metadata, rather than the cached resolve size24, governs IO.
    std::fs::write(&file.path, b"new").unwrap();
    guest.put(DESTINATION - ARG, &[0x5a; 40]);
    assert_eq!(submit(&mut owner, &mut guest), Ok(0));
    assert_eq!(guest.get(DESTINATION, 3), b"new");
    assert_eq!(guest.get(DESTINATION + 3, 37), [0x5a; 37]);
    assert_eq!(guest.get(ID, 4), 2_u32.to_le_bytes());
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::AmprReset, &[OBJECT]),
        Ok(0)
    );
    assert_eq!(owner.usage().ampr_commands, 0);
    assert_eq!(owner.usage().apr_submissions, 2);
    // Wait republishes its saved result after reset and consumes only that ID.
    guest.put(RESULT - ARG, &[0xff; 8]);
    assert_eq!(wait(&mut owner, &mut guest, (7 << 32) | 1), Ok(0));
    completion(&guest, 0, 0);
    assert_eq!(wait(&mut owner, &mut guest, 1), Ok(u64::from(u32::MAX)));
    assert_eq!(guest.get(errno, 4), 3_u32.to_le_bytes());
    assert_eq!(wait(&mut owner, &mut guest, 2), Ok(0));
    assert_eq!(owner.usage().apr_submissions, 0);

    append(&mut owner, &mut guest, 1, DESTINATION, 8, 3);
    guest.put(DESTINATION - ARG, &[0x91; 8]);
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::AprSubmitCommandBuffer,
            &[OBJECT, 123]
        ),
        Ok(0)
    );
    assert_eq!(guest.get(DESTINATION, 8), [0x91; 8]);
    assert_eq!(owner.usage().apr_submissions, 0); // Plain submit has no receipt.
}

#[test]
fn apr_submit_preserves_successful_prefix_reports_inner_failure_and_stops_following_reads() {
    let (_file, mut owner, mut guest) = setup(Default::default());
    append(&mut owner, &mut guest, 1, DESTINATION, 8, 4);
    append(&mut owner, &mut guest, 99, DESTINATION + 8, 8, 0);
    append(&mut owner, &mut guest, 1, DESTINATION + 16, 8, 0);
    guest.put(DESTINATION - ARG, &[0xa5; 32]);
    assert_eq!(submit(&mut owner, &mut guest), Ok(0));
    assert_eq!(guest.get(DESTINATION, 8), b"inal-dat");
    assert_eq!(guest.get(DESTINATION + 8, 24), [0xa5; 24]);
    completion(&guest, Errno::NoEntry.kernel_code(), 20);
    guest.put(RESULT - ARG, &[0; 8]);
    assert_eq!(wait(&mut owner, &mut guest, 1), Ok(0));
    completion(&guest, Errno::NoEntry.kernel_code(), 20);
    assert_eq!(owner.usage().ampr_commands, 3);
}

#[test]
fn apr_submit_outputs_follow_io_and_wait_removes_receipt_before_saved_result_failure() {
    let (_file, mut owner, mut guest) = setup(Default::default());
    append(&mut owner, &mut guest, 1, DESTINATION, 4, 0);
    // The result overwrites the overlapping ID only after that ID was stored.
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::AprSubmitCommandBufferAndGetResult,
            &[OBJECT, 0, RESULT, RESULT + 2]
        ),
        Ok(0)
    );
    completion(&guest, 0, 0);
    assert_eq!(wait(&mut owner, &mut guest, 1), Ok(0));
    guest.put(DESTINATION - ARG, &[0xa5; 8]);
    let errno = invoke(&mut owner, &mut guest, Service::ErrnoLocation, &[]).unwrap();
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::AprSubmitCommandBufferAndGetResult,
            &[OBJECT, 0, u64::MAX - 3, ID]
        ),
        Ok(u64::from(u32::MAX))
    );
    assert_eq!(guest.get(DESTINATION, 4), b"orig");
    assert_eq!(guest.get(ID, 4), 2_u32.to_le_bytes());
    assert_eq!(guest.get(errno, 4), 14_u32.to_le_bytes());
    assert_eq!(owner.usage().apr_submissions, 1);
    assert_eq!(wait(&mut owner, &mut guest, 2), Ok(u64::from(u32::MAX)));
    assert_eq!(owner.usage().apr_submissions, 0);
    assert_eq!(wait(&mut owner, &mut guest, 2), Ok(u64::from(u32::MAX)));
    assert_eq!(guest.get(errno, 4), 3_u32.to_le_bytes());
}

#[test]
fn apr_submit_outer_failure_retains_initial_receipt_and_policy_refusal_does_not_execute() {
    let (_file, mut owner, mut guest) = setup(Limits {
        max_apr_submissions: 1,
        ..Default::default()
    });
    let errno = invoke(&mut owner, &mut guest, Service::ErrnoLocation, &[]).unwrap();
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::AprSubmitCommandBufferAndGetId,
            &[OBJECT, 0, 0]
        ),
        Ok(u64::from(u32::MAX))
    );
    assert_eq!(owner.usage().apr_submissions, 0);
    assert_eq!(guest.get(errno, 4), 22_u32.to_le_bytes());
    // Header numeric overflow is an outer EFAULT after receipt allocation.
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::AprSubmitCommandBufferAndGetResult,
            &[u64::MAX, 0, RESULT, ID]
        ),
        Ok(u64::from(u32::MAX))
    );
    assert_eq!(owner.usage().apr_submissions, 1);
    guest.put(RESULT - ARG, &[0xff; 8]);
    append(&mut owner, &mut guest, 1, DESTINATION, 4, 0);
    guest.put(DESTINATION - ARG, &[0xa5; 8]);
    assert_eq!(
        submit(&mut owner, &mut guest),
        Err(Failure::Limit("APR submission receipts"))
    );
    assert_eq!(guest.get(DESTINATION, 8), [0xa5; 8]);
    assert_eq!(wait(&mut owner, &mut guest, 1), Ok(0));
    completion(&guest, 0, 0);
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::AprSubmitCommandBufferAndGetId,
            &[OBJECT, u64::MAX, ID]
        ),
        Ok(0)
    );
    assert_eq!(guest.get(ID, 4), 2_u32.to_le_bytes());
    assert_eq!(wait(&mut owner, &mut guest, 2), Ok(0));
}

#[test]
fn apr_submit_empty_cold_queue_accepts_null_backing_but_known_null_destination_fails_before_eof() {
    let (_file, mut owner, mut guest) = setup(Default::default());
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::AprSubmitCommandBufferAndGetResult,
            &[ARG + 0x600, u64::MAX, 0, 0]
        ),
        Ok(0)
    );
    assert_eq!(owner.usage().apr_submissions, 1);
    append(&mut owner, &mut guest, 1, 0, 1, 1 << 32);
    assert_eq!(submit(&mut owner, &mut guest), Ok(0));
    completion(&guest, Errno::Fault.kernel_code(), 0);
    assert_eq!(wait(&mut owner, &mut guest, 1), Ok(0));
    assert_eq!(wait(&mut owner, &mut guest, 2), Ok(0));
}
