# Development Notes

## Remove the patched usb-device dependency after an upstream release

As checked on 2026-10-26, the latest published `usb-device` version is 0.3.2,
which does not support the 512-byte control transfer buffer needed here.
`vendor/usb-device` therefore remains a submodule with a local
`control-buffer-512` backport.

Upstream [PR #144](https://github.com/rust-embedded-community/usb-device/pull/144)
already adds caller-provided control buffers, but this change is still unreleased.
When a release includes it:

- Update `usb-device` and pass a 512-byte buffer to `UsbDeviceBuilder` using the new API.
- Remove `control-buffer-512`, the `[patch.crates-io]` override, and the submodule.
- Update the README clone instructions and verify all three firmware targets and host tests.

The next release version and date were not confirmed.
