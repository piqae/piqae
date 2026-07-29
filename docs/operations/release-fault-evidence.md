# Release fault evidence

**Status:** deterministic local fault coverage implemented; managed-service,
multi-process, physical-printer, and fleet-soak evidence remains open.

No test in this document sends work to a physical printer.

## Evidence automated in the repository

Run:

```console
cargo test -p spool-object-store --test stream_faults --locked
cargo test -p spool-agent-storage --test offline_recovery --locked
```

The streaming suite proves that the filesystem object adapter:

- does not replace a previously verified object when the incoming byte stream
  disconnects;
- removes partial files after a stream, digest, or length failure;
- does not publish a new object until its declared length and SHA-256 match;
- writes and subsequently stream-verifies an 8 MiB document split across 128
  chunks.

The offline suite exercises distinct process-restart windows around local
acceptance. It proves that:

- a cloud job is not runnable before the server confirms its persisted
  acceptance intent;
- the exact pending acceptance survives restart without exposing its lease
  token through `Debug`;
- activation is idempotent;
- one runnable local job and one outbound `queued_local` event survive a later
  offline restart;
- the server event cursor can be acknowledged after reconnect and that
  acknowledgement survives another restart.

These tests use temporary files, SQLite, and virtual content only. They do not
prove S3/GCS multipart behavior, PostgreSQL failover, native spooler behavior,
or physical output.

## Evidence still required

Before promoting the related features from Preview to Supported, record:

1. S3-compatible and GCS-compatible stream interruption at multiple chunk
   boundaries, including service timeout and credential expiry.
2. The full 50 MiB accepted boundary and 50 MiB plus one-byte rejection under
   constrained memory, with resident-memory evidence.
3. PostgreSQL process termination during target rerouting, proving the job is
   assigned to at most one binding and no routing attempt is partially
   recorded.
4. Node loss before download, during download, before acceptance response, and
   after durable local acceptance.
5. Kubernetes pod deletion during an active lease and object transfer.
6. N/N-1 server and node combinations through every restart window.
7. A long-running fleet soak with zero silently lost jobs and zero duplicate
   native handoffs.
8. Named macOS HP and Windows HP/OKI physical evidence using controlled
   fixtures.

An object digest proves document integrity. A durable acceptance proves that
one node owns delivery. Neither proves that ink reached paper.
