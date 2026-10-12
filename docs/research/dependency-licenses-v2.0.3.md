# v2.0.3 依赖许可元数据清单

新增更新依赖：Tauri updater 2.13.3、process 2.4.0（JS 与 Rust），许可为 MIT OR Apache-2.0。官方来源：[plugins-workspace](https://github.com/tauri-apps/plugins-workspace)。

本清单从本机已安装 npm package.json 与 Cargo.lock 对应缓存 Cargo.toml 的 license 字段提取，只记录包名、版本、许可，不含本机路径。npm 范围含开发/构建依赖及包内可见元数据，不等于只随发行二进制分发的集合；未下载的其他平台 optional npm 包未覆盖。Rust 列出整个锁文件，未获取的目标包保持待核查。元数据清单不等于完整逐依赖版权、再分发义务或法律审计；原有完整依赖许可审计缺口继续保留。

## npm 本机安装元数据

| 包 | 版本 | 许可声明 |
| --- | --- | --- |
| @jridgewell/resolve-uri | 3.1.2 | MIT |
| @jridgewell/sourcemap-codec | 1.6.0 | MIT |
| @jridgewell/trace-mapping | 0.3.31 | MIT |
| @oxc-project/types | 0.153.0 | MIT |
| @rolldown/binding-darwin-arm64 | 1.2.13 | MIT |
| @rolldown/pluginutils | 1.0.1 | MIT |
| @tauri-apps/api | 2.12.3 | Apache-2.0 OR MIT |
| @tauri-apps/cli | 2.12.2 | Apache-2.0 OR MIT |
| @tauri-apps/cli-darwin-arm64 | 2.12.2 | Apache-2.0 OR MIT |
| @tauri-apps/plugin-process | 2.4.0 | MIT OR Apache-2.0 |
| @tauri-apps/plugin-updater | 2.13.3 | MIT OR Apache-2.0 |
| @types/chai | 5.2.3 | MIT |
| @types/deep-eql | 4.0.2 | MIT |
| @types/estree | 1.0.9 | MIT |
| @types/react | 19.3.0 | MIT |
| @types/react-dom | 19.3.0 | MIT |
| @vitejs/plugin-react | 6.1.2 | MIT |
| @vitest/mocker | 5.0.3 | MIT |
| @vitest/spy | 5.0.3 | MIT |
| assertion-error | 2.0.1 | MIT |
| chai | 6.3.0 | MIT |
| csstype | 3.2.3 | MIT |
| detect-libc | 2.1.2 | Apache-2.0 |
| es-module-lexer | 2.3.2 | MIT |
| estree-walker | 3.0.3 | MIT |
| expect-type | 1.4.0 | Apache-2.0 |
| fdir | 6.5.0 | MIT |
| fsevents | 2.3.3 | MIT |
| lightningcss | 1.33.0 | MPL-2.0 |
| lightningcss-darwin-arm64 | 1.33.0 | MPL-2.0 |
| magic-string | 1.4.3 | MIT |
| nanoid | 3.3.20 | MIT |
| obug | 2.2.1 | MIT |
| picocolors | 1.1.1 | ISC |
| picomatch | 4.0.7 | MIT |
| postcss | 8.5.29 | MIT |
| prettier | 3.9.10 | MIT |
| react | 19.3.0 | MIT |
| react-dom | 19.3.0 | MIT |
| rolldown | 1.2.13 | MIT |
| scheduler | 0.28.0 | MIT |
| source-map-js | 1.2.2 | BSD-3-Clause |
| std-env | 4.3.0 | MIT |
| tinybench | 6.2.1 | MIT |
| tinyexec | 1.3.1 | MIT |
| tinyglobby | 0.2.17 | MIT |
| typescript | 5.9.3 | Apache-2.0 |
| vite | 8.3.4 | MIT |
| vitest | 5.0.3 | MIT |
| why-is-node-running | 3.2.1 | MIT |

## Rust 锁文件

| Crate | 版本 | 许可声明 |
| --- | --- | --- |
| adler2 | 2.0.1 | 0BSD OR MIT OR Apache-2.0 |
| aho-corasick | 1.1.5 | Unlicense OR MIT |
| alloc-no-stdlib | 3.0.0 | BSD-3-Clause |
| alloc-stdlib | 0.3.0 | BSD-3-Clause |
| android_system_properties | 0.1.6 | 未获取对应平台包元数据 |
| anyhow | 1.0.104 | MIT OR Apache-2.0 |
| arbitrary | 1.5.0 | 未获取对应平台包元数据 |
| atk | 0.18.2 | 未获取对应平台包元数据 |
| atk-sys | 0.18.2 | 未获取对应平台包元数据 |
| atomic-waker | 1.1.2 | Apache-2.0 OR MIT |
| autocfg | 1.5.1 | Apache-2.0 OR MIT |
| base64 | 0.21.7 | MIT OR Apache-2.0 |
| base64 | 0.22.1 | MIT OR Apache-2.0 |
| base64 | 0.23.1 | MIT OR Apache-2.0 |
| bit-set | 0.8.0 | Apache-2.0 OR MIT |
| bit-vec | 0.8.0 | Apache-2.0 OR MIT |
| bitflags | 1.3.2 | MIT/Apache-2.0 |
| bitflags | 2.13.2 | MIT OR Apache-2.0 |
| block-buffer | 0.10.4 | MIT OR Apache-2.0 |
| block2 | 0.6.2 | MIT |
| brotli | 9.0.0 | BSD-3-Clause AND MIT |
| brotli-decompressor | 6.0.1 | BSD-3-Clause/MIT |
| bs58 | 0.5.1 | MIT/Apache-2.0 |
| bumpalo | 3.20.3 | 未获取对应平台包元数据 |
| bytemuck | 1.25.2 | 未获取对应平台包元数据 |
| byteorder | 1.5.0 | Unlicense OR MIT |
| bytes | 1.12.1 | MIT |
| cairo-rs | 0.18.5 | 未获取对应平台包元数据 |
| cairo-sys-rs | 0.18.2 | 未获取对应平台包元数据 |
| camino | 1.2.6 | MIT OR Apache-2.0 |
| cargo-platform | 0.1.9 | MIT OR Apache-2.0 |
| cargo_metadata | 0.19.2 | MIT |
| cargo_toml | 1.0.1 | Apache-2.0 OR MIT |
| cc | 1.7.0 | MIT OR Apache-2.0 |
| cesu8 | 1.1.0 | 未获取对应平台包元数据 |
| cfb | 0.14.0 | MIT |
| cfg-expr | 0.15.8 | 未获取对应平台包元数据 |
| cfg-if | 1.0.5 | MIT OR Apache-2.0 |
| cfg_aliases | 0.2.2 | MIT |
| chacha20 | 0.10.2 | MIT OR Apache-2.0 |
| chrono | 0.4.45 | MIT OR Apache-2.0 |
| combine | 4.6.8 | 未获取对应平台包元数据 |
| cookie | 0.18.2 | MIT OR Apache-2.0 |
| core-foundation | 0.10.1 | MIT OR Apache-2.0 |
| core-foundation | 0.9.4 | MIT OR Apache-2.0 |
| core-foundation-sys | 0.8.7 | MIT OR Apache-2.0 |
| core-graphics | 0.25.0 | MIT OR Apache-2.0 |
| core-graphics-types | 0.2.0 | MIT OR Apache-2.0 |
| cpufeatures | 0.2.17 | MIT OR Apache-2.0 |
| cpufeatures | 0.3.1 | 未获取对应平台包元数据 |
| crc32fast | 1.5.2 | MIT OR Apache-2.0 |
| crossbeam-channel | 0.5.17 | MIT OR Apache-2.0 |
| crossbeam-utils | 0.8.23 | MIT OR Apache-2.0 |
| crypto-common | 0.1.7 | MIT OR Apache-2.0 |
| cssparser | 0.37.0 | MPL-2.0 |
| cssparser-macros | 0.7.1 | MPL-2.0 |
| ctor | 1.0.13 | Apache-2.0 OR MIT |
| darling | 0.24.1 | MIT |
| darling_core | 0.24.1 | MIT |
| darling_macro | 0.24.1 | MIT |
| dbus | 0.9.12 | 未获取对应平台包元数据 |
| defmt | 1.1.1 | MIT OR Apache-2.0 |
| defmt-macros | 1.1.1 | MIT OR Apache-2.0 |
| defmt-parser | 1.0.0 | MIT OR Apache-2.0 |
| deranged | 0.5.8 | MIT OR Apache-2.0 |
| derive_arbitrary | 1.5.0 | 未获取对应平台包元数据 |
| derive_more | 2.1.1 | MIT |
| derive_more-impl | 2.1.1 | MIT |
| digest | 0.10.7 | MIT OR Apache-2.0 |
| dirs | 7.0.0 | MIT OR Apache-2.0 |
| dirs-sys | 0.5.0 | MIT OR Apache-2.0 |
| dispatch2 | 0.3.1 | Zlib OR Apache-2.0 OR MIT |
| displaydoc | 0.2.7 | MIT OR Apache-2.0 |
| dlopen2 | 0.8.2 | 未获取对应平台包元数据 |
| dlopen2_derive | 0.4.3 | 未获取对应平台包元数据 |
| dom_query | 0.28.0 | MIT |
| dpi | 0.1.2 | Apache-2.0 AND MIT |
| dtoa | 1.0.11 | MIT OR Apache-2.0 |
| dtoa-short | 0.3.5 | MPL-2.0 |
| dunce | 1.0.5 | CC0-1.0 OR MIT-0 OR Apache-2.0 |
| dyn-clone | 1.0.20 | MIT OR Apache-2.0 |
| embed-resource | 3.0.12 | MIT |
| embed_plist | 1.2.2 | MIT OR Apache-2.0 |
| equivalent | 1.0.2 | Apache-2.0 OR MIT |
| erased-serde | 0.4.10 | MIT OR Apache-2.0 |
| errno | 0.3.14 | MIT OR Apache-2.0 |
| fallible-iterator | 0.3.0 | MIT/Apache-2.0 |
| fallible-streaming-iterator | 0.1.9 | MIT/Apache-2.0 |
| fastrand | 2.5.0 | Apache-2.0 OR MIT |
| fdeflate | 0.3.7 | MIT OR Apache-2.0 |
| field-offset | 0.3.6 | 未获取对应平台包元数据 |
| filetime | 0.2.29 | MIT/Apache-2.0 |
| find-msvc-tools | 0.1.14 | MIT OR Apache-2.0 |
| flate2 | 1.1.10 | MIT OR Apache-2.0 |
| fnv | 1.0.7 | Apache-2.0 / MIT |
| foldhash | 0.1.5 | Zlib |
| foldhash | 0.2.0 | Zlib |
| foreign-types | 0.5.0 | MIT/Apache-2.0 |
| foreign-types-macros | 0.2.4 | MIT/Apache-2.0 |
| foreign-types-shared | 0.3.1 | MIT/Apache-2.0 |
| form_urlencoded | 1.2.2 | MIT OR Apache-2.0 |
| fsevent-sys | 4.1.0 | MIT |
| futures-channel | 0.3.34 | MIT OR Apache-2.0 |
| futures-core | 0.3.34 | MIT OR Apache-2.0 |
| futures-executor | 0.3.34 | 未获取对应平台包元数据 |
| futures-io | 0.3.34 | MIT OR Apache-2.0 |
| futures-macro | 0.3.34 | MIT OR Apache-2.0 |
| futures-sink | 0.3.34 | MIT OR Apache-2.0 |
| futures-task | 0.3.34 | MIT OR Apache-2.0 |
| futures-util | 0.3.34 | MIT OR Apache-2.0 |
| gdk | 0.18.2 | 未获取对应平台包元数据 |
| gdk-pixbuf | 0.18.5 | 未获取对应平台包元数据 |
| gdk-pixbuf-sys | 0.18.0 | 未获取对应平台包元数据 |
| gdk-sys | 0.18.2 | 未获取对应平台包元数据 |
| gdkwayland-sys | 0.18.2 | 未获取对应平台包元数据 |
| gdkx11 | 0.18.2 | 未获取对应平台包元数据 |
| gdkx11-sys | 0.18.2 | 未获取对应平台包元数据 |
| generic-array | 0.14.7 | MIT |
| getrandom | 0.2.17 | MIT OR Apache-2.0 |
| getrandom | 0.3.4 | MIT OR Apache-2.0 |
| getrandom | 0.4.3 | MIT OR Apache-2.0 |
| gio | 0.18.4 | 未获取对应平台包元数据 |
| gio-sys | 0.18.1 | 未获取对应平台包元数据 |
| glib | 0.18.5 | 未获取对应平台包元数据 |
| glib-macros | 0.18.5 | 未获取对应平台包元数据 |
| glib-sys | 0.18.1 | 未获取对应平台包元数据 |
| glob | 0.3.4 | MIT OR Apache-2.0 |
| gobject-sys | 0.18.0 | 未获取对应平台包元数据 |
| gtk | 0.18.2 | 未获取对应平台包元数据 |
| gtk-sys | 0.18.2 | 未获取对应平台包元数据 |
| gtk3-macros | 0.18.2 | 未获取对应平台包元数据 |
| hashbrown | 0.12.3 | MIT OR Apache-2.0 |
| hashbrown | 0.15.5 | MIT OR Apache-2.0 |
| hashbrown | 0.17.1 | MIT OR Apache-2.0 |
| hashlink | 0.10.0 | MIT OR Apache-2.0 |
| heck | 0.4.1 | 未获取对应平台包元数据 |
| heck | 0.5.0 | MIT OR Apache-2.0 |
| hex | 0.4.3 | MIT OR Apache-2.0 |
| hmac | 0.12.1 | MIT OR Apache-2.0 |
| html5ever | 0.39.0 | MIT OR Apache-2.0 |
| http | 1.5.0 | MIT OR Apache-2.0 |
| http-body | 1.1.0 | MIT |
| http-body-util | 0.1.5 | MIT |
| httparse | 1.10.1 | MIT OR Apache-2.0 |
| hyper | 1.12.0 | MIT |
| hyper-rustls | 0.27.10 | Apache-2.0 OR ISC OR MIT |
| hyper-util | 0.1.21 | MIT |
| iana-time-zone | 0.1.65 | MIT OR Apache-2.0 |
| iana-time-zone-haiku | 0.1.2 | 未获取对应平台包元数据 |
| ico | 0.5.0 | MIT |
| icu_collections | 2.3.0 | Unicode-3.0 |
| icu_locale_core | 2.3.0 | Unicode-3.0 |
| icu_normalizer | 2.3.0 | Unicode-3.0 |
| icu_normalizer_data | 2.3.0 | Unicode-3.0 |
| icu_properties | 2.3.0 | Unicode-3.0 |
| icu_properties_data | 2.3.0 | Unicode-3.0 |
| icu_provider | 2.3.1 | Unicode-3.0 |
| ident_case | 1.0.1 | MIT/Apache-2.0 |
| idna | 1.1.0 | MIT OR Apache-2.0 |
| idna_adapter | 1.2.2 | Apache-2.0 OR MIT |
| indexmap | 1.9.3 | Apache-2.0 OR MIT |
| indexmap | 2.14.2 | Apache-2.0 OR MIT |
| infer | 0.22.0 | MIT |
| inotify | 0.11.5 | 未获取对应平台包元数据 |
| inotify-sys | 0.1.8 | 未获取对应平台包元数据 |
| ipnet | 2.12.2 | MIT OR Apache-2.0 |
| itoa | 1.0.18 | MIT OR Apache-2.0 |
| javascriptcore-rs | 1.1.2 | 未获取对应平台包元数据 |
| javascriptcore-rs-sys | 1.1.1 | 未获取对应平台包元数据 |
| jiff | 0.2.38 | Unlicense OR MIT |
| jiff-core | 0.1.1 | Unlicense OR MIT |
| jiff-static | 0.2.38 | 未获取对应平台包元数据 |
| jiff-tzdb | 0.1.9 | 未获取对应平台包元数据 |
| jiff-tzdb-platform | 0.1.3 | 未获取对应平台包元数据 |
| jni | 0.21.1 | 未获取对应平台包元数据 |
| jni | 0.22.4 | 未获取对应平台包元数据 |
| jni-macros | 0.22.4 | 未获取对应平台包元数据 |
| jni-sys | 0.3.1 | 未获取对应平台包元数据 |
| jni-sys | 0.4.1 | 未获取对应平台包元数据 |
| jni-sys-macros | 0.4.1 | 未获取对应平台包元数据 |
| jobserver | 0.1.35 | MIT OR Apache-2.0 |
| js-sys | 0.3.106 | 未获取对应平台包元数据 |
| json-patch | 4.2.0 | MIT/Apache-2.0 |
| jsonptr | 0.7.1 | MIT OR Apache-2.0 |
| keyboard-types | 0.8.3 | MIT OR Apache-2.0 |
| keyring | 3.6.3 | MIT OR Apache-2.0 |
| kqueue | 1.2.1 | 未获取对应平台包元数据 |
| kqueue-sys | 1.1.2 | 未获取对应平台包元数据 |
| libappindicator | 0.9.0 | 未获取对应平台包元数据 |
| libappindicator-sys | 0.9.0 | 未获取对应平台包元数据 |
| libc | 0.2.190 | MIT OR Apache-2.0 |
| libdbus-sys | 0.2.7 | 未获取对应平台包元数据 |
| libloading | 0.7.4 | 未获取对应平台包元数据 |
| libredox | 0.1.25 | 未获取对应平台包元数据 |
| libsqlite3-sys | 0.35.0 | MIT |
| linux-raw-sys | 0.12.1 | 未获取对应平台包元数据 |
| litemap | 0.8.3 | Unicode-3.0 |
| lock_api | 0.4.14 | MIT OR Apache-2.0 |
| log | 0.4.34 | MIT OR Apache-2.0 |
| lru-slab | 0.1.3 | MIT OR Apache-2.0 OR Zlib |
| markup5ever | 0.39.0 | MIT OR Apache-2.0 |
| memchr | 2.8.3 | Unlicense OR MIT |
| memoffset | 0.9.1 | 未获取对应平台包元数据 |
| mime | 0.3.17 | MIT OR Apache-2.0 |
| minisign-verify | 0.3.0 | MIT |
| miniz_oxide | 0.8.9 | MIT OR Zlib OR Apache-2.0 |
| miniz_oxide | 0.9.1 | MIT OR Zlib OR Apache-2.0 |
| mio | 1.2.4 | MIT |
| muda | 0.20.0 | Apache-2.0 OR MIT |
| ndk | 0.9.0 | 未获取对应平台包元数据 |
| ndk-context | 0.1.1 | 未获取对应平台包元数据 |
| ndk-sys | 0.6.0+11769913 | 未获取对应平台包元数据 |
| new_debug_unreachable | 1.0.6 | MIT |
| notify | 8.2.0 | CC0-1.0 |
| notify-types | 2.1.0 | MIT OR Apache-2.0 |
| num-conv | 0.2.2 | MIT OR Apache-2.0 |
| num-traits | 0.2.19 | MIT OR Apache-2.0 |
| num_enum | 0.7.6 | 未获取对应平台包元数据 |
| num_enum_derive | 0.7.6 | 未获取对应平台包元数据 |
| objc2 | 0.6.5 | MIT |
| objc2-app-kit | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-cloud-kit | 0.3.2 | 未获取对应平台包元数据 |
| objc2-core-data | 0.3.2 | 未获取对应平台包元数据 |
| objc2-core-foundation | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-core-graphics | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-core-image | 0.3.2 | 未获取对应平台包元数据 |
| objc2-core-location | 0.3.2 | 未获取对应平台包元数据 |
| objc2-core-text | 0.3.2 | 未获取对应平台包元数据 |
| objc2-encode | 4.1.0 | MIT |
| objc2-exception-helper | 0.1.1 | Zlib OR Apache-2.0 OR MIT |
| objc2-foundation | 0.3.2 | MIT |
| objc2-io-surface | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-osa-kit | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-quartz-core | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-ui-kit | 0.3.2 | 未获取对应平台包元数据 |
| objc2-user-notifications | 0.3.2 | 未获取对应平台包元数据 |
| objc2-web-kit | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| once_cell | 1.21.4 | MIT OR Apache-2.0 |
| openssl-probe | 0.2.1 | 未获取对应平台包元数据 |
| option-ext | 0.2.0 | MPL-2.0 |
| osakit | 0.3.1 | MIT OR Apache-2.0 |
| pango | 0.18.3 | 未获取对应平台包元数据 |
| pango-sys | 0.18.0 | 未获取对应平台包元数据 |
| parking_lot | 0.12.5 | MIT OR Apache-2.0 |
| parking_lot_core | 0.9.12 | MIT OR Apache-2.0 |
| percent-encoding | 2.3.2 | MIT OR Apache-2.0 |
| phf | 0.13.1 | MIT |
| phf_codegen | 0.13.1 | MIT |
| phf_generator | 0.13.1 | MIT |
| phf_macros | 0.13.1 | MIT |
| phf_shared | 0.13.1 | MIT |
| pin-project-lite | 0.2.17 | Apache-2.0 OR MIT |
| pkg-config | 0.3.34 | MIT OR Apache-2.0 |
| plist | 1.10.1 | MIT |
| png | 0.17.16 | MIT OR Apache-2.0 |
| png | 0.18.1 | MIT OR Apache-2.0 |
| portable-atomic | 1.15.0 | 未获取对应平台包元数据 |
| portable-atomic-util | 0.2.8 | 未获取对应平台包元数据 |
| potential_utf | 0.1.6 | Unicode-3.0 |
| powerfmt | 0.2.1 | MIT OR Apache-2.0 |
| precomputed-hash | 0.1.1 | MIT |
| proc-macro-crate | 1.3.1 | MIT OR Apache-2.0 |
| proc-macro-crate | 2.0.2 | 未获取对应平台包元数据 |
| proc-macro-crate | 3.5.0 | MIT OR Apache-2.0 |
| proc-macro-error | 1.0.4 | 未获取对应平台包元数据 |
| proc-macro-error-attr | 1.0.4 | 未获取对应平台包元数据 |
| proc-macro2 | 1.0.107 | MIT OR Apache-2.0 |
| quick-xml | 0.42.0 | MIT |
| quinn | 0.11.12 | MIT OR Apache-2.0 |
| quinn-proto | 0.11.19 | MIT OR Apache-2.0 |
| quinn-udp | 0.5.16 | MIT OR Apache-2.0 |
| quote | 1.0.47 | MIT OR Apache-2.0 |
| r-efi | 5.3.0 | 未获取对应平台包元数据 |
| r-efi | 6.0.0 | 未获取对应平台包元数据 |
| rand | 0.10.3 | MIT OR Apache-2.0 |
| rand_core | 0.10.1 | MIT OR Apache-2.0 |
| rand_pcg | 0.10.2 | MIT OR Apache-2.0 |
| raw-window-handle | 0.6.2 | MIT OR Apache-2.0 OR Zlib |
| redox_syscall | 0.5.18 | 未获取对应平台包元数据 |
| redox_users | 0.5.3 | 未获取对应平台包元数据 |
| ref-cast | 1.0.27 | MIT OR Apache-2.0 |
| ref-cast-impl | 1.0.27 | MIT OR Apache-2.0 |
| regex | 1.13.1 | MIT OR Apache-2.0 |
| regex-automata | 0.4.18 | MIT OR Apache-2.0 |
| regex-syntax | 0.8.11 | MIT OR Apache-2.0 |
| reqwest | 0.12.28 | MIT OR Apache-2.0 |
| reqwest | 0.13.5 | MIT OR Apache-2.0 |
| ring | 0.17.14 | Apache-2.0 AND ISC |
| rusqlite | 0.37.0 | MIT |
| rustc-hash | 2.1.3 | Apache-2.0 OR MIT |
| rustc_version | 0.4.1 | MIT OR Apache-2.0 |
| rustix | 1.1.5 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| rustls | 0.23.45 | Apache-2.0 OR ISC OR MIT |
| rustls-native-certs | 0.8.4 | 未获取对应平台包元数据 |
| rustls-pki-types | 1.15.1 | MIT OR Apache-2.0 |
| rustls-platform-verifier | 0.7.1 | MIT OR Apache-2.0 |
| rustls-platform-verifier-android | 0.2.0 | 未获取对应平台包元数据 |
| rustls-webpki | 0.103.15 | ISC |
| rustversion | 1.0.23 | 未获取对应平台包元数据 |
| ryu | 1.0.23 | Apache-2.0 OR BSL-1.0 |
| same-file | 1.0.6 | Unlicense/MIT |
| schannel | 0.1.29 | 未获取对应平台包元数据 |
| schemars | 0.8.22 | MIT |
| schemars | 0.9.0 | MIT |
| schemars | 1.2.2 | MIT |
| schemars_derive | 0.8.22 | MIT |
| scopeguard | 1.2.0 | MIT OR Apache-2.0 |
| security-framework | 2.11.1 | 未获取对应平台包元数据 |
| security-framework | 3.7.0 | MIT OR Apache-2.0 |
| security-framework-sys | 2.17.0 | MIT OR Apache-2.0 |
| selectors | 0.38.0 | MPL-2.0 |
| semver | 1.0.28 | MIT OR Apache-2.0 |
| serde | 1.0.229 | MIT OR Apache-2.0 |
| serde-untagged | 0.1.9 | MIT OR Apache-2.0 |
| serde_core | 1.0.229 | MIT OR Apache-2.0 |
| serde_derive | 1.0.229 | MIT OR Apache-2.0 |
| serde_derive_internals | 0.29.1 | MIT OR Apache-2.0 |
| serde_json | 1.0.152 | MIT OR Apache-2.0 |
| serde_repr | 0.1.21 | MIT OR Apache-2.0 |
| serde_spanned | 0.6.9 | MIT OR Apache-2.0 |
| serde_spanned | 1.1.2 | MIT OR Apache-2.0 |
| serde_urlencoded | 0.7.1 | MIT/Apache-2.0 |
| serde_with | 3.24.0 | MIT OR Apache-2.0 |
| serde_with_macros | 3.24.0 | MIT OR Apache-2.0 |
| serialize-to-javascript | 0.1.2 | MIT OR Apache-2.0 |
| serialize-to-javascript-impl | 0.1.2 | MIT OR Apache-2.0 |
| servo_arc | 0.4.3 | MIT OR Apache-2.0 |
| sha2 | 0.10.9 | MIT OR Apache-2.0 |
| shlex | 2.0.1 | MIT OR Apache-2.0 |
| simd-adler32 | 0.3.10 | MIT |
| simd_cesu8 | 1.2.0 | 未获取对应平台包元数据 |
| simdutf8 | 0.1.5 | MIT OR Apache-2.0 |
| siphasher | 1.0.4 | MIT OR Apache-2.0 |
| slab | 0.4.12 | MIT |
| smallvec | 1.16.3 | MIT OR Apache-2.0 |
| socket2 | 0.6.5 | MIT OR Apache-2.0 |
| softbuffer | 0.4.8 | 未获取对应平台包元数据 |
| soup3 | 0.5.0 | 未获取对应平台包元数据 |
| soup3-sys | 0.5.0 | 未获取对应平台包元数据 |
| stable_deref_trait | 1.2.1 | MIT OR Apache-2.0 |
| string_cache | 0.9.0 | MIT OR Apache-2.0 |
| string_cache_codegen | 0.6.1 | MIT OR Apache-2.0 |
| strsim | 0.11.1 | MIT |
| subtle | 2.6.1 | BSD-3-Clause |
| swift-rs | 1.0.9 | MIT OR Apache-2.0 |
| syn | 1.0.109 | MIT OR Apache-2.0 |
| syn | 2.0.119 | MIT OR Apache-2.0 |
| syn | 3.0.7 | MIT OR Apache-2.0 |
| sync_wrapper | 1.0.2 | Apache-2.0 |
| synstructure | 0.14.0 | MIT |
| system-configuration | 0.7.0 | MIT OR Apache-2.0 |
| system-configuration-sys | 0.6.0 | MIT OR Apache-2.0 |
| system-deps | 6.2.2 | 未获取对应平台包元数据 |
| tao | 0.37.1 | Apache-2.0 |
| tao-macros | 0.1.4 | 未获取对应平台包元数据 |
| tar | 0.4.46 | MIT OR Apache-2.0 |
| target-lexicon | 0.12.16 | 未获取对应平台包元数据 |
| tauri | 2.12.3 | Apache-2.0 OR MIT |
| tauri-build | 2.7.2 | Apache-2.0 OR MIT |
| tauri-codegen | 2.7.2 | Apache-2.0 OR MIT |
| tauri-macros | 2.7.2 | Apache-2.0 OR MIT |
| tauri-plugin | 2.7.1 | Apache-2.0 OR MIT |
| tauri-plugin-process | 2.4.0 | Apache-2.0 OR MIT |
| tauri-plugin-updater | 2.13.3 | Apache-2.0 OR MIT |
| tauri-runtime | 2.12.1 | Apache-2.0 OR MIT |
| tauri-runtime-wry | 2.12.1 | Apache-2.0 OR MIT |
| tauri-utils | 2.10.1 | Apache-2.0 OR MIT |
| tauri-winres | 0.3.6 | MIT |
| tempfile | 3.27.0 | MIT OR Apache-2.0 |
| tendril | 0.5.1 | MIT OR Apache-2.0 |
| thiserror | 1.0.69 | MIT OR Apache-2.0 |
| thiserror | 2.0.21 | MIT OR Apache-2.0 |
| thiserror-impl | 1.0.69 | MIT OR Apache-2.0 |
| thiserror-impl | 2.0.21 | MIT OR Apache-2.0 |
| time | 0.3.55 | MIT OR Apache-2.0 |
| time-core | 0.1.9 | MIT OR Apache-2.0 |
| time-macros | 0.2.32 | MIT OR Apache-2.0 |
| tinystr | 0.8.4 | Unicode-3.0 |
| tinyvec | 1.13.3 | Zlib OR Apache-2.0 OR MIT |
| tokio | 1.53.2 | MIT |
| tokio-macros | 2.7.2 | MIT |
| tokio-rustls | 0.26.6 | MIT OR Apache-2.0 |
| tokio-util | 0.7.20 | MIT |
| toml | 0.8.2 | 未获取对应平台包元数据 |
| toml | 1.1.8+spec-1.1.0 | MIT OR Apache-2.0 |
| toml_datetime | 0.6.3 | 未获取对应平台包元数据 |
| toml_datetime | 1.1.2+spec-1.1.0 | MIT OR Apache-2.0 |
| toml_edit | 0.19.15 | MIT OR Apache-2.0 |
| toml_edit | 0.20.2 | 未获取对应平台包元数据 |
| toml_edit | 0.25.17+spec-1.1.0 | 未获取对应平台包元数据 |
| toml_parser | 1.1.5+spec-1.1.0 | MIT OR Apache-2.0 |
| toml_writer | 1.1.3+spec-1.1.0 | MIT OR Apache-2.0 |
| tower | 0.5.3 | MIT |
| tower-http | 0.6.11 | MIT |
| tower-layer | 0.3.3 | MIT |
| tower-service | 0.3.3 | MIT |
| tracing | 0.1.44 | MIT |
| tracing-core | 0.1.36 | MIT |
| tray-icon | 0.25.1 | MIT OR Apache-2.0 |
| try-lock | 0.2.5 | MIT |
| typeid | 1.0.3 | MIT OR Apache-2.0 |
| typenum | 1.20.1 | MIT OR Apache-2.0 |
| unicode-ident | 1.0.26 | (MIT OR Apache-2.0) AND Unicode-3.0 |
| unicode-segmentation | 1.13.3 | 未获取对应平台包元数据 |
| untrusted | 0.9.0 | ISC |
| url | 2.5.8 | MIT OR Apache-2.0 |
| urlpattern | 0.6.0 | MIT |
| utf8_iter | 1.0.4 | Apache-2.0 OR MIT |
| uuid | 1.28.0 | Apache-2.0 OR MIT |
| vcpkg | 0.2.15 | MIT/Apache-2.0 |
| version-compare | 0.2.1 | 未获取对应平台包元数据 |
| version_check | 0.9.5 | MIT/Apache-2.0 |
| vswhom | 0.1.0 | 未获取对应平台包元数据 |
| vswhom-sys | 0.1.3 | 未获取对应平台包元数据 |
| walkdir | 2.5.0 | Unlicense/MIT |
| want | 0.3.2 | MIT |
| wasi | 0.11.1+wasi-snapshot-preview1 | 未获取对应平台包元数据 |
| wasip2 | 1.0.4+wasi-0.2.12 | 未获取对应平台包元数据 |
| wasm-bindgen | 0.2.129 | 未获取对应平台包元数据 |
| wasm-bindgen-futures | 0.4.79 | 未获取对应平台包元数据 |
| wasm-bindgen-macro | 0.2.129 | 未获取对应平台包元数据 |
| wasm-bindgen-macro-support | 0.2.129 | 未获取对应平台包元数据 |
| wasm-bindgen-shared | 0.2.129 | 未获取对应平台包元数据 |
| wasm-streams | 0.5.0 | 未获取对应平台包元数据 |
| web-sys | 0.3.106 | 未获取对应平台包元数据 |
| web-time | 1.1.0 | MIT OR Apache-2.0 |
| web_atoms | 0.2.6 | MIT OR Apache-2.0 |
| webkit2gtk | 2.0.2 | 未获取对应平台包元数据 |
| webkit2gtk-sys | 2.0.2 | 未获取对应平台包元数据 |
| webpki-root-certs | 1.0.9 | 未获取对应平台包元数据 |
| webpki-roots | 1.0.9 | CDLA-Permissive-2.0 |
| webview2-com | 0.39.1 | 未获取对应平台包元数据 |
| webview2-com-macros | 0.8.1 | 未获取对应平台包元数据 |
| webview2-com-sys | 0.39.1 | 未获取对应平台包元数据 |
| winapi | 0.3.9 | 未获取对应平台包元数据 |
| winapi-i686-pc-windows-gnu | 0.4.0 | 未获取对应平台包元数据 |
| winapi-util | 0.1.11 | 未获取对应平台包元数据 |
| winapi-x86_64-pc-windows-gnu | 0.4.0 | 未获取对应平台包元数据 |
| window-vibrancy | 0.8.1 | Apache-2.0 OR MIT |
| windows | 0.62.2 | 未获取对应平台包元数据 |
| windows-collections | 0.3.2 | 未获取对应平台包元数据 |
| windows-core | 0.62.2 | 未获取对应平台包元数据 |
| windows-future | 0.3.2 | 未获取对应平台包元数据 |
| windows-implement | 0.60.2 | 未获取对应平台包元数据 |
| windows-interface | 0.59.3 | 未获取对应平台包元数据 |
| windows-link | 0.2.1 | 未获取对应平台包元数据 |
| windows-numerics | 0.3.1 | 未获取对应平台包元数据 |
| windows-registry | 0.6.1 | 未获取对应平台包元数据 |
| windows-result | 0.4.1 | 未获取对应平台包元数据 |
| windows-strings | 0.5.1 | 未获取对应平台包元数据 |
| windows-sys | 0.45.0 | 未获取对应平台包元数据 |
| windows-sys | 0.52.0 | 未获取对应平台包元数据 |
| windows-sys | 0.60.2 | 未获取对应平台包元数据 |
| windows-sys | 0.61.2 | 未获取对应平台包元数据 |
| windows-targets | 0.42.2 | 未获取对应平台包元数据 |
| windows-targets | 0.52.6 | 未获取对应平台包元数据 |
| windows-targets | 0.53.5 | 未获取对应平台包元数据 |
| windows-threading | 0.2.1 | 未获取对应平台包元数据 |
| windows-version | 0.1.7 | 未获取对应平台包元数据 |
| windows_aarch64_gnullvm | 0.42.2 | 未获取对应平台包元数据 |
| windows_aarch64_gnullvm | 0.52.6 | 未获取对应平台包元数据 |
| windows_aarch64_gnullvm | 0.53.1 | 未获取对应平台包元数据 |
| windows_aarch64_msvc | 0.42.2 | 未获取对应平台包元数据 |
| windows_aarch64_msvc | 0.52.6 | 未获取对应平台包元数据 |
| windows_aarch64_msvc | 0.53.1 | 未获取对应平台包元数据 |
| windows_i686_gnu | 0.42.2 | 未获取对应平台包元数据 |
| windows_i686_gnu | 0.52.6 | 未获取对应平台包元数据 |
| windows_i686_gnu | 0.53.1 | 未获取对应平台包元数据 |
| windows_i686_gnullvm | 0.52.6 | 未获取对应平台包元数据 |
| windows_i686_gnullvm | 0.53.1 | 未获取对应平台包元数据 |
| windows_i686_msvc | 0.42.2 | 未获取对应平台包元数据 |
| windows_i686_msvc | 0.52.6 | 未获取对应平台包元数据 |
| windows_i686_msvc | 0.53.1 | 未获取对应平台包元数据 |
| windows_x86_64_gnu | 0.42.2 | 未获取对应平台包元数据 |
| windows_x86_64_gnu | 0.52.6 | 未获取对应平台包元数据 |
| windows_x86_64_gnu | 0.53.1 | 未获取对应平台包元数据 |
| windows_x86_64_gnullvm | 0.42.2 | 未获取对应平台包元数据 |
| windows_x86_64_gnullvm | 0.52.6 | 未获取对应平台包元数据 |
| windows_x86_64_gnullvm | 0.53.1 | 未获取对应平台包元数据 |
| windows_x86_64_msvc | 0.42.2 | 未获取对应平台包元数据 |
| windows_x86_64_msvc | 0.52.6 | 未获取对应平台包元数据 |
| windows_x86_64_msvc | 0.53.1 | 未获取对应平台包元数据 |
| winnow | 0.5.40 | MIT |
| winnow | 1.0.4 | MIT |
| winreg | 0.56.0 | 未获取对应平台包元数据 |
| wit-bindgen | 0.57.1 | 未获取对应平台包元数据 |
| writeable | 0.6.4 | Unicode-3.0 |
| wry | 0.57.0 | Apache-2.0 OR MIT |
| x11 | 2.21.0 | 未获取对应平台包元数据 |
| x11-dl | 2.21.0 | 未获取对应平台包元数据 |
| xattr | 1.6.1 | MIT OR Apache-2.0 |
| yoke | 0.8.3 | Unicode-3.0 |
| yoke-derive | 0.8.4 | Unicode-3.0 |
| zerofrom | 0.1.8 | Unicode-3.0 |
| zerofrom-derive | 0.1.8 | Unicode-3.0 |
| zeroize | 1.9.1 | Apache-2.0 OR MIT |
| zerotrie | 0.2.5 | Unicode-3.0 |
| zerovec | 0.11.8 | Unicode-3.0 |
| zerovec-derive | 0.11.6 | Unicode-3.0 |
| zip | 4.6.1 | 未获取对应平台包元数据 |
| zlib-rs | 0.6.8 | Zlib |
| zmij | 1.0.23 | MIT |
| zstd | 0.14.2 | BSD-3-Clause |
| zstd-safe | 8.1.0 | BSD-3-Clause |
| zstd-sys | 2.1.1+zstd.1.5.7 | BSD-3-Clause |
