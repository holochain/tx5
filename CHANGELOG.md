# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2025-06-19

### Added

- Add doc comments to tx5 crate (#151) by @neonphog in [#151](https://github.com/holochain/tx5/pull/151)
- Add doc comments for tx5-connection (#150) by @neonphog in [#150](https://github.com/holochain/tx5/pull/150)
- Add doc comments to go-pion-sys crate (#149) by @neonphog in [#149](https://github.com/holochain/tx5/pull/149)
- Add stress tests and fix broken pipe when send too fast (#124) by @neonphog in [#124](https://github.com/holochain/tx5/pull/124)
- Address unbounded channel in signal client (#70) by @neonphog in [#70](https://github.com/holochain/tx5/pull/70)
- Add todo for hack removal (#67) by @neonphog in [#67](https://github.com/holochain/tx5/pull/67)
- Add conn preflight and validate hooks (#27) by @neonphog in [#27](https://github.com/holochain/tx5/pull/27)
- Add broadcast api (#26) by @neonphog in [#26](https://github.com/holochain/tx5/pull/26)

### Changed

- Add release workflows by @ThetaSinner in [#153](https://github.com/holochain/tx5/pull/153)
- Update dependencies by @ThetaSinner
- 2 passing and 2 ignored failing event tests (#152) by @neonphog in [#152](https://github.com/holochain/tx5/pull/152)
- Add ci_pass check (#148) by @ThetaSinner in [#148](https://github.com/holochain/tx5/pull/148)
- Cleanup docs (#147) by @neonphog in [#147](https://github.com/holochain/tx5/pull/147)
- Bump version by @neonphog
- Bump version to 0.4.2 by @ThetaSinner
- Get connected URLs from the endpoint (#142) by @ThetaSinner in [#142](https://github.com/holochain/tx5/pull/142)
- Bump ver by @neonphog
- Authenticate (#140) by @neonphog in [#140](https://github.com/holochain/tx5/pull/140)
- Preload (#141) by @neonphog in [#141](https://github.com/holochain/tx5/pull/141)
- Bump version to 0.4.0 (#138) by @ThetaSinner in [#138](https://github.com/holochain/tx5/pull/138)
- Make configuration typed (#137) by @ThetaSinner in [#137](https://github.com/holochain/tx5/pull/137)
- Ignore flaky test by @neonphog
- Ignore flaky test by @neonphog
- Bump version by @neonphog
- Updated datachannel (#136) by @guillemcordoba in [#136](https://github.com/holochain/tx5/pull/136)
- Bump sbd libraries by @ThetaSinner
- Bump version to 0.3.2 (#134) by @ThetaSinner in [#134](https://github.com/holochain/tx5/pull/134)
- Bump to 0.3.1-beta (#133) by @matthme in [#133](https://github.com/holochain/tx5/pull/133)
- Bump sbd to 0.0.11-alpha2 (#132) by @matthme in [#132](https://github.com/holochain/tx5/pull/132)
- Maint 03 25 (#131) by @ThetaSinner in [#131](https://github.com/holochain/tx5/pull/131)
- Bump version by @neonphog
- Make vendored feature for libdatachannel optional (#126) by @ThetaSinner in [#126](https://github.com/holochain/tx5/pull/126)
- Only use sbd messaging as a fallback for webrtc failure (#127) by @neonphog in [#127](https://github.com/holochain/tx5/pull/127)
- Bump version by @neonphog
- Bump version by @neonphog
- Bump sbd (#118) by @neonphog in [#118](https://github.com/holochain/tx5/pull/118)
- Repo (#117) by @neonphog in [#117](https://github.com/holochain/tx5/pull/117)
- Bump sbd and tx5 versions by @neonphog
- Libdatachannel backend impl (#111) by @neonphog in [#111](https://github.com/holochain/tx5/pull/111)
- Lint by @neonphog
- Bump version by @neonphog
- Mem backend for tx5 (#106) by @neonphog in [#106](https://github.com/holochain/tx5/pull/106)
- Abstract tx5 backend in prep for m̶o̶c̶k̶  mem backend (#105) by @neonphog in [#105](https://github.com/holochain/tx5/pull/105)
- Configurable slow_app_timeout defaulting to ~1s by @neonphog
- Bump version by @neonphog
- Connection buffer and close fixes (#104) by @neonphog in [#104](https://github.com/holochain/tx5/pull/104)
- Bump version by @neonphog
- Multi-sig tests and fixes (#101) by @neonphog in [#101](https://github.com/holochain/tx5/pull/101)
- Bump 0.1.0-beta (#100) by @neonphog in [#100](https://github.com/holochain/tx5/pull/100)
- Use app_dirs2 UserCache directory for storing go lib by default, overridable via env variable (#97) by @mattyg in [#97](https://github.com/holochain/tx5/pull/97)
- Netaudit (#95) by @neonphog in [#95](https://github.com/holochain/tx5/pull/95)
- Vcpkg (#96) by @neonphog in [#96](https://github.com/holochain/tx5/pull/96)
- Bump versions by @neonphog
- Ensure SigUrls and PeerUrls are always in a canonical form with even well-known ports listed explicitly (#94) by @neonphog in [#94](https://github.com/holochain/tx5/pull/94)
- Expose webrtc config (#93) by @neonphog in [#93](https://github.com/holochain/tx5/pull/93)
- Publish order by @neonphog
- Bump version by @neonphog
- Webrtc upgrade for tx5 connections (#92) by @neonphog in [#92](https://github.com/holochain/tx5/pull/92)
- Sbd integration into tx5 (#91) by @neonphog in [#91](https://github.com/holochain/tx5/pull/91)
- Bump to 0.0.9-alpha (#88) by @ThetaSinner in [#88](https://github.com/holochain/tx5/pull/88)
- Allow connections to be closed from the public API (#84) by @ThetaSinner in [#84](https://github.com/holochain/tx5/pull/84)
- Various tx5 fixes (#82) by @neonphog in [#82](https://github.com/holochain/tx5/pull/82)
- `musl-gcc` support (#81) by @ThetaSinner in [#81](https://github.com/holochain/tx5/pull/81)
- Refactor of endpoint and connection management state logic (#77) by @neonphog in [#77](https://github.com/holochain/tx5/pull/77)
- Warnings fix + turn_doctor: actually use calculated ports (#80) by @neonphog in [#80](https://github.com/holochain/tx5/pull/80)
- Bump patch version to 0.0.6-alpha (#78) by @jost-s in [#78](https://github.com/holochain/tx5/pull/78)
- Bump lair_keystore_api (#76) by @jost-s in [#76](https://github.com/holochain/tx5/pull/76)
- Don't check the go version on docs.rs builds (#75) by @neonphog in [#75](https://github.com/holochain/tx5/pull/75)
- Fix cross-compiling from darwin to ios (#73) by @guillemcordoba in [#73](https://github.com/holochain/tx5/pull/73)
- Bump versions (#66) by @neonphog in [#66](https://github.com/holochain/tx5/pull/66)
- Switch to static linking go code for targets that support it (#11) by @neonphog in [#11](https://github.com/holochain/tx5/pull/11)
- Bump influxive (#64) by @ThetaSinner in [#64](https://github.com/holochain/tx5/pull/64)
- Turn-doctor (#61) by @neonphog in [#61](https://github.com/holochain/tx5/pull/61)
- Build/go disable buildvcs (#60) by @mattyg in [#60](https://github.com/holochain/tx5/pull/60)
- Bump versions (#59) by @neonphog in [#59](https://github.com/holochain/tx5/pull/59)
- Allow config of pion udp ephemeral port range (#55) by @neonphog in [#55](https://github.com/holochain/tx5/pull/55)
- Update stale.yml by @neonphog
- Stale (#57) by @neonphog in [#57](https://github.com/holochain/tx5/pull/57)
- Bump min go version to 1.20 & re-vendor (#56) by @neonphog in [#56](https://github.com/holochain/tx5/pull/56)
- Android ci (#52) by @neonphog in [#52](https://github.com/holochain/tx5/pull/52)
- Test rust 1.66.1 which is currently used by holochain; fixes for rust 1.66.1; add nix development shell (#48) by @steveej in [#48](https://github.com/holochain/tx5/pull/48)
- New prerelease versioning scheme + bump lair version (#49) by @neonphog in [#49](https://github.com/holochain/tx5/pull/49)
- Send buffer does not grow indefinitely (#47) by @neonphog in [#47](https://github.com/holochain/tx5/pull/47)
- Update go pion library (#46) by @neonphog in [#46](https://github.com/holochain/tx5/pull/46)
- Influxive (#45) by @neonphog in [#45](https://github.com/holochain/tx5/pull/45)
- Benchmark (#44) by @neonphog in [#44](https://github.com/holochain/tx5/pull/44)
- New tx5-demo (#43) by @neonphog in [#43](https://github.com/holochain/tx5/pull/43)
- Bump versions by @neonphog
- Bump versions by @neonphog
- Use webpki-roots on systems where native-certs doesn't work (#35) by @neonphog in [#35](https://github.com/holochain/tx5/pull/35)
- 'Text file busy' fix (#33) by @neonphog in [#33](https://github.com/holochain/tx5/pull/33)
- Bump versions by @neonphog
- Harden lib loading against race conditions (#32) by @neonphog in [#32](https://github.com/holochain/tx5/pull/32)
- Bump versions by @neonphog
- Stats and fixes (#31) by @neonphog in [#31](https://github.com/holochain/tx5/pull/31)
- Bump versions by @neonphog
- Update signal config for usage in hc command (#30) by @neonphog in [#30](https://github.com/holochain/tx5/pull/30)
- Update if-addrs and other deps (#28) by @neonphog in [#28](https://github.com/holochain/tx5/pull/28)
- Bump versions by @neonphog
- Bump versions by @neonphog
- Bump versions by @neonphog
- Ability to ban remote ids in endpoint (in lieu of connection close) (#25) by @neonphog in [#25](https://github.com/holochain/tx5/pull/25)
- Bump versions by @neonphog
- Get stats from peer connection (#24) by @neonphog in [#24](https://github.com/holochain/tx5/pull/24)
- Sig Seq Validation (#23) by @neonphog in [#23](https://github.com/holochain/tx5/pull/23)
- Forward go logging to rust tracing (#22) by @neonphog in [#22](https://github.com/holochain/tx5/pull/22)
- Version bumps by @neonphog
- Signal server versioning (#21) by @neonphog in [#21](https://github.com/holochain/tx5/pull/21)
- Sig connect example (#20) by @neonphog in [#20](https://github.com/holochain/tx5/pull/20)
- Bump versions by @neonphog
- Signal Connection Management Fixes (#18) by @neonphog in [#18](https://github.com/holochain/tx5/pull/18)
- First cut at online status utility (#19) by @neonphog in [#19](https://github.com/holochain/tx5/pull/19)
- Turn stress (#17) by @neonphog in [#17](https://github.com/holochain/tx5/pull/17)
- Chunk + multiplex messages to stay under webrtc msg size limits (#16) by @neonphog in [#16](https://github.com/holochain/tx5/pull/16)
- Tx5 Test Fixes (#15) by @neonphog in [#15](https://github.com/holochain/tx5/pull/15)
- Bump versions by @neonphog
- Tracing && Doc Updates (#14) by @neonphog in [#14](https://github.com/holochain/tx5/pull/14)
- Mitigate some security issues with shared object loading (#12) by @neonphog in [#12](https://github.com/holochain/tx5/pull/12)
- Docker update by @neonphog
- Debugging linux ci (#10) by @neonphog in [#10](https://github.com/holochain/tx5/pull/10)
- Make fix by @neonphog
- Pion turn (#9) by @neonphog in [#9](https://github.com/holochain/tx5/pull/9)
- No go debug symbols by @neonphog
- Version bumps by @neonphog
- Gocache under out_dir by @neonphog
- Bump version by @neonphog
- Cargo.lock by @neonphog
- Bump go-pion-sys version by @neonphog
- Back up to go 1.18 (#8) by @neonphog in [#8](https://github.com/holochain/tx5/pull/8)
- Cleanup by @neonphog
- Build.rs fix by @neonphog
- Rename by @neonphog
- Integration work (#7) by @neonphog in [#7](https://github.com/holochain/tx5/pull/7)
- Doc fix by @neonphog
- Lint by @neonphog
- Merge pull request #6 from holochain/demo by @neonphog in [#6](https://github.com/holochain/tx5/pull/6)
- Clippy by @neonphog
- Cleanup by @neonphog
- Checkpoint by @neonphog
- Update dep by @neonphog
- Update deps by @neonphog
- Demo by @neonphog
- Resurrect tx4-demo by @neonphog
- Workspace the path refs too by @neonphog
- Merge pull request #4 from holochain/tx4-high-level by @neonphog in [#4](https://github.com/holochain/tx5/pull/4)
- Config by @neonphog
- Connection rotation by @neonphog
- Some connection cleanup code by @neonphog
- Start on metrics by @neonphog
- Tx4 high level sends data by @neonphog
- Checkpoint by @neonphog
- Checkpoint by @neonphog
- Checkpoint by @neonphog
- Impolite offer test by @neonphog
- Polite negotiation test by @neonphog
- Incoming offer by @neonphog
- Passing by @neonphog
- Checkpoint by @neonphog
- Checkpoint by @neonphog
- Checkpoint by @neonphog
- Checkpoint by @neonphog
- Checkpoint by @neonphog
- Snathoue by @neonphog
- Checkpoint by @neonphog
- Checkpoint by @neonphog
- Checkpoint by @neonphog
- Merge pull request #3 from holochain/integration by @neonphog in [#3](https://github.com/holochain/tx5/pull/3)
- Cleanup by @neonphog
- Docs by @neonphog
- Checkpoint by @neonphog
- Checkpoint by @neonphog
- Checkpoint integration crate by @neonphog
- Prometheus by @neonphog
- Docker fix by @neonphog
- Merge pull request #2 from holochain/pion-cleanup by @neonphog in [#2](https://github.com/holochain/tx5/pull/2)
- Codegen for constants by @neonphog
- Update go vendor dir by @neonphog
- Async high-level lib by @neonphog
- Tracing from go by @neonphog
- Cleanup rust->go json handling by @neonphog
- Go version by @neonphog
- Go fmt version by @neonphog
- Checkpoint by @neonphog
- Go fmt by @neonphog
- Checkpoint by @neonphog
- Checkpoint by @neonphog
- Checkpoint by @neonphog
- Checkpoint by @neonphog
- Rm tls api by @neonphog
- Checkpoint by @neonphog
- Error cleanup by @neonphog
- Merge pull request #1 from holochain/sig-cleanup by @neonphog in [#1](https://github.com/holochain/tx5/pull/1)
- Signal test by @neonphog
- Checkpoint by @neonphog
- Checkpoint by @neonphog
- Checkpoint by @neonphog
- Checkpoint by @neonphog
- Checkpoint new server architecture by @neonphog
- Checkpoint by @neonphog
- Checkpoint by @neonphog
- Cleanup sig-srv config by @neonphog
- Signal reorg by @neonphog
- Checkpoint signal server by @neonphog
- Checkpoint sig-cleanup by @neonphog
- Workaround windows filename issue by @neonphog
- Signal srv docker by @neonphog
- Demo by @neonphog
- Signal-core cleanup by @neonphog
- Import signal by @neonphog
- Initial commit by @neonphog

### Fixed

- Prevent connecting to self (#102) by @jost-s in [#102](https://github.com/holochain/tx5/pull/102)
- Fix tests when run on android (#63) by @neonphog in [#63](https://github.com/holochain/tx5/pull/63)
- Fix turn-stress by @neonphog
- Fix turn-stress by @neonphog
- Fix initial message connect timeout & properly report disconnects (#42) by @neonphog in [#42](https://github.com/holochain/tx5/pull/42)
- Fix typo by @neonphog
- Fix signal client tls (#13) by @neonphog in [#13](https://github.com/holochain/tx5/pull/13)
- Test fix by @neonphog
- Test fix by @neonphog
- Fix feature by @neonphog
- Test fix by @neonphog
- Test fix by @neonphog

### Removed

- Remove the app timeout (#116) by @neonphog in [#116](https://github.com/holochain/tx5/pull/116)
- Remove workaround to test rustix fix for tempfile::persist_noclobber (#68) by @neonphog in [#68](https://github.com/holochain/tx5/pull/68)
- Remove defunct test by @neonphog

## First-time Contributors

* @ThetaSinner made their first contribution in [#153](https://github.com/holochain/tx5/pull/153)

* @neonphog made their first contribution in [#152](https://github.com/holochain/tx5/pull/152)

* @jost-s made their first contribution in [#102](https://github.com/holochain/tx5/pull/102)

* @guillemcordoba made their first contribution in [#136](https://github.com/holochain/tx5/pull/136)

* @matthme made their first contribution in [#133](https://github.com/holochain/tx5/pull/133)

* @mattyg made their first contribution in [#97](https://github.com/holochain/tx5/pull/97)

* @steveej made their first contribution in [#48](https://github.com/holochain/tx5/pull/48)


<!-- generated by git-cliff -->
