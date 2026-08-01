# Artifact reference v1

Schema identity: `urn:terminal-interop:artifact-ref:v1`.

The reference contract lets a producer name one completed local artifact without putting a long,
platform-specific path into the interaction surface.

## Representations

- Short: `@TOKEN`
- URI: `terminal-interop://artifact/TOKEN`
- JSON: the complete `ArtifactRefV1` record emitted by `term-interop offer --format json`

`TOKEN` is 13 characters from a case-insensitive, ambiguity-reduced Base32 alphabet. It is an
opaque lookup identity, not a path encoding and not a content capability transferable between
machines.

## Registration transition

An entry can be constructed only when all of these facts hold:

1. the supplied path resolves to a regular file;
2. the file is no larger than the v1 32 MiB input bound;
3. the exact canonical path bytes can be represented by the platform profile;
4. the source can be read completely and hashed;
5. a private registry entry and its `latest` pointer can be persisted atomically.

The record binds size, modification time, device and inode where the platform exposes them, and
SHA-256 of the exact bytes. Resolution opens the path again and revalidates the same identity. A
missing or changed file produces an error; a token never silently follows replacement content.

## Ownership and portability

The registry is local state owned by the user who created it. Discovery order is:

1. `TERM_INTEROP_STATE_DIR` (exact registry root),
2. `XDG_STATE_HOME/terminal-interop`,
3. `HOME/.local/state/terminal-interop`.

Exact Unix path bytes are Base64-encoded as `unix-bytes-v1`. Non-Unix platforms use the explicit
`utf8-v1` profile. A consumer must reject an encoding it does not implement.

The URI is a typed handoff for applications that know how to reach the same registry. It does not
authorize a desktop launcher, shell, network service, or remote host by itself.
