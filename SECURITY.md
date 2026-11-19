# Security policy

## Current scope

v0.2 is a trusted-LAN research release, not an Internet-ready conferencing service. It has no authentication, room access control, TURN deployment, or TLS termination in the core. The test client uses a self-signed HTTPS development certificate. Media transport uses WebRTC ICE/DTLS/SRTP.

Input limits reduce resource abuse but do not replace authentication or a security audit. Do not expose the signaling or media ports to an untrusted network. There is no long-term support commitment yet; fixes target the current development version.

## Reporting a vulnerability

Use [GitHub private vulnerability reporting](https://github.com/AlpaguDevelopment/Alpagu-SFU/security/advisories/new). Include affected versions, reproduction steps, and impact. Remove credentials and personal data from attachments. Do not publish exploit details in a public issue before coordinated disclosure.

If private reporting is unavailable, open an issue requesting a private contact channel without disclosing the vulnerability itself.
