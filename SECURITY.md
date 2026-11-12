# Security policy

Halyard handles end-to-end encrypted messages and their keys, so security reports matter a lot.

## Reporting a vulnerability

**Don't open a public issue.** Report privately through GitHub:
[Security → Report a vulnerability](https://github.com/aamirsahmad/halyard/security/advisories/new).

Please include:

* what the problem is and what an attacker could do with it;
* the steps or a proof of concept to reproduce it;
* the affected version or commit, and your operating system.

We aim to acknowledge reports within a few days and to agree a disclosure date with you. We'll
credit you in the advisory unless you'd rather we didn't.

## What's in scope

* Halyard's own code: local storage encryption (the SQLCipher databases, the attachment store
  and key handling), how protocol content is processed, the UI, and the packaging.
* How Halyard uses presage, libsignal-service-rs and libsignal.

Vulnerabilities in those libraries themselves belong to their projects:

* [libsignal](https://github.com/signalapp/libsignal) and the Signal protocol or servers: see
  [Signal's security page](https://signal.org/security/).
* [presage](https://github.com/whisperfish/presage) and
  [libsignal-service-rs](https://github.com/whisperfish/libsignal-service-rs): those
  repositories.

If you're not sure where a problem belongs, report it here and we'll help route it.

## Supported versions

Only the latest release gets security fixes.
