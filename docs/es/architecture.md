# xpm — Arquitectura

Arquitectura técnica de `xpm`, basada en el layout del workspace, el código de las crates bajo
`crates/`, el README y los documentos de esta carpeta.

---

## Workspace y crates

El repositorio es un Cargo workspace con dos miembros:

```
xpm/
├── Cargo.toml                  # raíz del workspace (resolver 2, deps compartidas, versión 0.1.0)
├── crates/
│   ├── xpm/                    # crate binaria — frontend CLI
│   │   └── src/
│   │       ├── main.rs         # entry point, logging, carga de config, despacho de subcomandos
│   │       └── cli.rs          # definiciones CLI con clap (comandos, aliases, flags)
│   └── xpm-core/               # crate librería — lógica core
│       └── src/
│           ├── lib.rs          # raíz de módulos y re-exports
│           ├── config.rs       # parser de configuración TOML
│           ├── error.rs        # tipos ConfigError / XpmError
│           ├── repo.rs         # gestor de repositorios (repos de usuario en /etc/xpm.d/)
│           ├── repo_db.rs      # parsing de .db / .files estilo ALPM
│           ├── repo_sync.rs    # descarga / sync remoto de bases de datos
│           ├── signing.rs      # verificación de firmas OpenPGP separadas
│           ├── hooks.rs        # trait Hook + hooks integrados de transacción
│           ├── transaction.rs  # motor de transacciones (plan/prepare/commit)
│           ├── package/        # lectores y parsers de .xp / .pkg.tar.zst
│           │   ├── reader.rs, pkginfo.rs, buildinfo.rs, mtree.rs,
│           │   ├── types.rs, validate.rs
│           └── resolver/       # resolución de dependencias basada en SAT
│               ├── version.rs, dependency.rs, types.rs, provider.rs
├── etc/xpm.conf.example        # configuración de ejemplo
├── tests/                      # tests de integración de base de datos de repo
├── ROADMAP.md                  # roadmap de desarrollo interno
└── docs/                       # documentación (esta carpeta)
```

La crate binaria (`xpm`) depende solo de `clap`, `anyhow`, `tracing`, `tracing-subscriber` y
`xpm-core`. Toda la lógica de dominio vive en `xpm-core`.

## Flujo de despacho del CLI

`main.rs` sigue un pipeline lineal:

1. Parseo del CLI con clap (`Cli::parse`).
2. Inicialización del logging con `tracing`; la verbosidad `-v`/`-vv`/`-vvv` se asigna a
   INFO/DEBUG/TRACE (por defecto WARN). `RUST_LOG` lo sobrescribe vía `EnvFilter`.
3. Carga de la configuración desde `--config` (por defecto `/etc/xpm.conf`); si el archivo falta,
   el parser cae a los defaults integrados (`load_or_default`).
4. Aplicación de los overrides del CLI para `--root`, `--dbpath`, `--cachedir` y `--no-color`.
5. Despacho al handler del subcomando parseado.

## Resolución de dependencias

El resolver vive en `crates/xpm-core/src/resolver/` y delega toda la mecánica SAT a la crate
`resolvo` (CDCL, watched literals, unit propagation). xpm aporta el modelo de datos y la
implementación de `DependencyProvider`. Cuatro módulos:

| Módulo | Archivo | Propósito |
|--------|---------|-----------|
| `version` | `version.rs` | Parsing/comparación de versiones compatible con `vercmp` de ALPM |
| `dependency` | `dependency.rs` | Parsing y matching de constraints de dependencia |
| `types` | `types.rs` | Package pool y puente de interning hacia resolvo |
| `provider` | `provider.rs` | `XpmProvider` implementando `Interner` + `DependencyProvider` de resolvo |

Tipos clave:

- `Version` — versión ALPM parseada `[epoch:]pkgver[-pkgrel]`; comparación segmento a segmento
  (numérico vs. alfabético) acorde a `vercmp`.
- `Operator` — `Ge (>=)`, `Le (<=)`, `Gt (>)`, `Lt (<)`, `Eq (=)`.
- `DepConstraint` — una dependencia parseada como `glibc>=2.38` (`name`, `op` opcional,
  `version` opcional); `matches(&Version)` evalúa un candidato.
- `PackageCandidate` — una versión instalable con `name`, `version`, `depends`, `conflicts`,
  `provides`, `optdepends`.
- `PackagePool` — arena de interning que asigna nombres, candidatos, version sets, uniones de
  version sets y strings a los IDs opacos de resolvo. Los conflictos usan
  `intern_conflict_version_set`, que guarda el constraint con `negated = true` para que resolvo
  prohíba a los candidatos que *sí* matchean.
- `XpmProvider` — alimenta el pool al solver (`filter_candidates`, `get_candidates`,
  `sort_candidates` de mayor a menor, `get_dependencies` asignando `depends` a requirements y
  `conflicts` a constrains).

El README expresa el modelo como cláusulas CNF:

| Requisito | Cláusula CNF | Significado |
|-----------|--------------|-------------|
| Dependencia | `!foo OR bar` | Si `foo` está instalado, `bar` debe estarlo también |
| Requisito raíz | `foo` | El paquete objetivo es obligatorio |
| Conflicto | `!bar_v1 OR !bar_v2` | Versiones mutuamente excluyentes |

Nota honesta: el resolver está implementado y testeado a nivel de librería (ver
`docs/RESOLVER.md`), pero los handlers actuales de `install`/`upgrade` del CLI en `main.rs` no lo
invocan — `install` selecciona un paquete por nombre exacto desde la base de datos sincronizada y
`upgrade` compara versiones con `Version::cmp_versions`. Usar el solver desde el CLI es trabajo
futuro.

## Formato de paquete

xpm lee paquetes `.xp` de forma nativa y mantiene compatibilidad con los `.pkg.tar.zst` de Arch.
Ambos son archivos `tar.zst` (compresión zstd; el lector también autodetecta gzip y xz por magic
bytes) que contienen entradas de metadatos:

| Entrada | Contenido |
|---------|-----------|
| `.PKGINFO` | Metadatos `key = value`: nombre, versión+release, descripción, URL, fecha de build, tamaño, arch, licencias, `depends`, `makedepends`, `checkdepends`, `optdepends`, `provides`, `conflicts`, `replaces` |
| `.BUILDINFO` | Registro del entorno de build reproducible |
| `.MTREE` | Manifest de archivos con modos, tamaños y hashes SHA-256 (`sha256digest=`) |
| `.INSTALL` | Scriptlets bash opcionales (`pre_install`, `post_install`, `pre_upgrade`, `post_upgrade`, `pre_remove`, `post_remove`) |

`crates/xpm-core/src/package/reader.rs` ofrece `read_metadata`, `list_files`, `extract_to` y
`read_raw_entry`; los parsers viven en `pkginfo.rs`, `buildinfo.rs` y `mtree.rs`; los structs
compartidos en `types.rs`. `validate.rs` implementa la validación de integridad
post-extracción: los archivos extraídos se comprueban contra el manifest `.MTREE` en checksums
SHA-256, tamaños y tipos de archivo.

## Base de datos de repositorio y sync

`repo_db.rs` parsea las bases de datos de repositorio estilo ALPM:

- `RepoEntry` — un paquete con metadatos más las extensiones opcionales de xpkg: `filename`,
  `sha256sum`, `url` y una lista `files`.
- `SyncDb` / `LocalDb` — base de datos de sync parseada y la base de datos local de instalados.
- `parse_sync_db`, `merge_files_db` — parsean el archivo `.db` y fusionan la lista `.files`
  opcional.

`repo_sync.rs` maneja las operaciones remotas:

- `expand_repo_url` — sustituye `$repo` y `$arch` en las URLs de mirrors.
- `sync_repo_databases` — descarga `<repo>.db` (obligatorio) y `<repo>.files` (best-effort) al
  directorio de sync, probando los mirrors en orden con reintentos.
- `verify_remote_signature` / `verify_sha256` — enforcement de firma y checksum.
- `package_download_candidates` / `download_first_available` — construyen URLs candidatas para
  un paquete y descargan desde el primer mirror alcanzable.

Las bases de datos de sync caen bajo `<db_path>/sync/` (p.ej. `/var/lib/xpm/sync/<repo>.db`). La
base de datos local de paquetes instalados vive bajo `<db_path>/local/<pkg>/` y guarda al menos
un archivo `version` y un manifest `files`.

## Firma

`signing.rs` usa `sequoia-openpgp` (Rust puro) para verificar firmas OpenPGP separadas:

- `load_keyring` — parsea un archivo de keyring (p.ej. `trustedkeys.gpg`) en un conjunto de
  certificados.
- `verify_file` / `verify_detached` — verifica una firma separada sobre los bytes de un archivo.
- `VerifyOutcome` — `Good { key_id }`, `UnknownKey` o `Bad { reason }`.

El enforcement lo controla `sig_level` de la configuración (con override por repositorio):
`required` falla ante firmas ausentes/inválidas, `optional` avisa y `never` omite la
verificación.

## Transacciones y hooks

`transaction.rs` implementa el motor de transacciones con un ciclo de vida
plan/prepare/commit:

- `FileLock` — lock de filesystem (bajo la base de datos local) que impide operaciones xpm
  concurrentes.
- `TransactionOp` — `Install { pkg_name, pkg_version, pkg_file }`, `Remove { pkg_name }`,
  `Upgrade { ... }`.
- `TransactionState` — `Planning`, `Prepared`, `Committed`, `RolledBack`.
- `Transaction` — recoge operaciones durante el planning, ejecuta el pre-flight de prepare y
  luego commitea; los fallos disparan lógica de rollback; las operaciones se añaden al log
  (por defecto `/var/log/xpm.log` bajo la raíz).

`hooks.rs` ofrece el trait `Hook` y un `HookChain`. La cadena por defecto ejecuta, por operación:

1. `MetadataLoadHook` — carga los metadatos del paquete para la operación.
2. `PreScriptletHook` — ejecuta los pre-scriptlets de `.INSTALL` (`pre_install`, `pre_upgrade`,
   `pre_remove`).
3. `FileExtractionHook` — extrae los archivos del paquete con la propiedad correcta.
4. `FileRemovalHook` — elimina los archivos trackeados al desinstalar (y poda directorios vacíos
   fuera de la raíz del sistema).
5. `PostScriptletHook` — ejecuta los post-scriptlets de `.INSTALL` (`post_install`,
   `post_upgrade`, `post_remove`).
6. `LocalDbHook` — registra/elimina el paquete en la base de datos local.

Los scriptlets se ejecutan vía `bash` con las variables `XPM_ROOT_DIR`, `XPM_PKG_NAME` y
`XPM_PKG_VERSION` exportadas.

## Configuración

`/etc/xpm.conf` es un archivo TOML con `[options]` y un bloque `[[repo]]` por repositorio
predefinido. Los defaults provienen de `config.rs` y `etc/xpm.conf.example`:

```toml
[options]
root_dir = "/"
db_path = "/var/lib/xpm/"
cache_dir = "/var/cache/xpm/pkg/"
log_file = "/var/log/xpm.log"
gpg_dir = "/etc/pacman.d/gnupg/"     # default del código
sig_level = "optional"               # optional | required | never
color = true
parallel_downloads = 5
check_space = true
# architecture = "x86_64"            # autodetectado si no se define
# hold_pkg = ["linux"]
# ignore_pkg = [...]
# ignore_group = [...]

[[repo]]
name = "x"
server = ["https://equislinux.github.io/x-repo/x/$arch"]
# sig_level = "required"             # override opcional por repositorio
```

Notas:

- Si el archivo no existe, xpm cae a los defaults integrados (`XpmConfig::default`), cuyo único
  repositorio es `x` en `https://equislinux.github.io/x-repo/x/$arch`.
- El archivo de ejemplo y algunos fragmentos de README/help difieren en el default del
  directorio GPG: el código usa `/etc/pacman.d/gnupg/`, mientras que la guía del README usa
  `/etc/xpm/gnupg/`. Conviene reconciliar esta discrepancia antes de depender de ella.
- La validación de config rechaza `parallel_downloads = 0`, nombres de repo vacíos y
  repositorios sin servidores.
- `sig_level` puede definirse globalmente o por repositorio; el valor por repo gana si está
  presente.
- Los repositorios añadidos por el usuario se guardan individualmente en
  `/etc/xpm.d/<name>.toml` y se gestionan con `xpm repo add/remove/list`.
- Las variables de URL `$repo` y `$arch` se sustituyen en el momento del fetch.

## Hosting del repositorio

El repositorio por defecto está en GitHub Pages en `equislinux.github.io/x-repo` y es accesible como
árbol estático; por eso xpm soporta cualquier servidor HTTP estático (la migración a un VPS es
transparente). El orden de los repositorios predefinidos en la config decide la prioridad de
paquetes (gana el primer repositorio que ofrezca el paquete); dentro de un repositorio, los
mirrors se prueban en orden hasta que uno responde.

## Dependencias externas (workspace)

| Área | Crates |
|------|--------|
| CLI | `clap` 4 |
| Serialización | `serde` 1, `toml` 0.8 |
| Errores | `anyhow` 1, `thiserror` 2 |
| Resolución | `resolvo` 0.10, `itertools` 0.14 |
| Archivos/compresión | `tar` 0.4, `zstd` 0.13, `flate2` 1, `xz2` 0.1 |
| Cripto | `sha2` 0.10, `sequoia-openpgp` 1 (rust crypto, sin features C por defecto) |
| Logging | `tracing` 0.1, `tracing-subscriber` 0.3 |
| Red | `reqwest` 0.12 (blocking, rustls-tls) |

## Referencias

- Detalles del resolver y cobertura de tests: [`../RESOLVER.md`](../RESOLVER.md)
- Objetivos de fetch y endpoints: [`../FETCH_TARGETS.md`](../FETCH_TARGETS.md)
- Integración con xpkg: [`../INTEGRATION.md`](../INTEGRATION.md)
- Config de ejemplo: [`../../etc/xpm.conf.example`](../../etc/xpm.conf.example)
