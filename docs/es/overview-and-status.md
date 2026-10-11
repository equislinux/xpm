# xpm — Resumen y estado

Este documento describe qué es `xpm`, dónde se sitúa dentro del ecosistema X y
su estado actual en el toolchain. Se basa en el contenido del repositorio tal y
como está hoy (README.md, ROADMAP.md, código, tests, docs/), no en un estado
futuro deseado.

---

## Qué es xpm

`xpm` es un gestor de paquetes moderno y de alto rendimiento, escrito en Rust
puro para la distribución X. Está diseñado como reemplazo nativo en Rust de
`pacman` y `libalpm`:

- Formato de paquete nativo `.xp` (X Package, un archivo `tar.zst`) con
  metadata `.PKGINFO`, `.BUILDINFO` y `.MTREE`.
- Compatibilidad con paquetes `.pkg.tar.zst` de Arch Linux y bases de
  repositorio estilo ALPM (`.db` / `.files`).
- Resolución de dependencias con SAT, impulsada por el crate `resolvo`.
- Archivo de configuración TOML en `/etc/xpm.conf`.
- Verificación de firmas detached OpenPGP con `sig_level` configurable
  (`required` / `optional` / `never`).
- Gestión de repositorios con repos predefinidos y repos de usuario en
  `/etc/xpm.d/`.

El proyecto pertenece a la organización `equislinux`. Los paquetes los
construye la herramienta compañera `xpkg` (repo `equislinux/xpkg`); `xpm`
consume lo que `xpkg` produce.

## Dónde se sitúa xpm

Cada componente vive en su propio repositorio de GitHub bajo `equislinux`:

| Carpeta   | Repo                  | Rol |
|-----------|-----------------------|-----|
| `x`       | `equislinux/x`        | La distro (build archiso de imagen/ISO y aprovisionamiento) |
| `scripts` | `equislinux/scripts`  | Payload de aprovisionamiento del sistema y CLI `x` |
| `xpkg`    | `equislinux/xpkg`     | Herramienta de empaquetado Rust (builder) |
| `xpm`     | `equislinux/xpm`      | Gestor de paquetes Rust (este repo) |
| `x-repo`  | `equislinux/x-repo`   | Repositorios de paquetes (pacman `[x]` y `.xp` nativo) y portales |

## Estado actual

`xpm` está conectado de extremo a extremo y el endpoint nativo `.xp` que sirve
está vivo:

- El resolver SAT corre dentro de `install` y `upgrade`: cierre de
  dependencias, orden de dependencias, requisitos `name=version` y `provides`
  sin versión.
- La instalación de `.xp` locales, la eliminación de paquetes y las
  actualizaciones con dependencias funcionan sobre el mismo motor de
  transacciones; `sync`/`search` leen las bases local y de sync.
- El endpoint en `https://equislinux.github.io/x-repo/x/$arch` (construido por
  `scripts/build-xp.sh` de `x-repo`, separado del endpoint pacman en
  `/repo/x86_64`) se validó con `xpm sync/search/deps/install` contra la URL
  publicada.
- El sistema se sigue aprovisionando con **pacman + el repositorio firmado
  `[x]`** — `xpm` coexiste como canal nativo en lugar de sustituir el flujo de
  producción.

Problemas abiertos conocidos (seguimiento en este repo):

1. Resolver: `Unsolvable(Conflict)` con paquetes que dependen de `gcc-libs`
   (ciclo de dependencias) incluso en un root limpio.
2. Extracción: falla la extracción de hardlinks en algunos paquetes reales de
   Arch (archivos `zoneinfo` bajo `usr/share/zoneinfo/`).

## Roadmap interno e hitos

`xpm` mantiene su propia ROADMAP.md con fases 0-10. Lo más relevante:

- Fases 0-1 completas: scaffolding, CLI con 8 subcomandos más `repo`/`usage`,
  configuración TOML.
- Fase 2 (puente FFI de libalpm) omitida por diseño — el proyecto fue directo a
  Rust nativo para evitar dependencias C.
- Fase 3 completa: resolver SAT con `resolvo`, comparación de versiones
  `vercmp` de ALPM, parseo de dependencias, manejo de conflictos, tests de
  integración.
- Fase 4 completa: lectores `.xp` / `.pkg.tar.zst`, parsers de metadata,
  extracción de archivos y validación de integridad post-instalación.
- Fase 5 mayormente completa: parseo de `.db` / `.files` de ALPM, base local
  bajo `/var/lib/xpm/local/`, sync remoto con descargas HTTP, backend de GitHub
  Pages, sustitución de variables en URLs.
- Fase 10, pista de seguridad: verificación de `.db.sig` y `.sig` de paquetes
  más carga de keyring.

La convención de versiones del roadmap del repo es:

| Versión  | Hito |
|----------|-----------|
| `v0.1.0` | Fases 0-1 completas (CLI funcional con configuración) |
| `v0.2.0` | Fase 2 omitida (sin dependencia de libalpm) |
| `v0.5.0` | Fases 3-5 completas (motor nativo operativo) |
| `v0.8.0` | Fases 6-7 completas (seguridad + transacciones) |
| `v1.0.0` | Fase 8 completa (con benchmarks, testeado, listo para producción) |

El `Cargo.toml` declara actualmente la versión `0.1.0`.

## Sobre el estado de los comandos

No todos los subcomandos están totalmente cableados al motor. De
`crates/xpm/src/main.rs`:

- `sync`, `install`, `remove`, `upgrade` y `repo` despachan a lógica real de
  transacciones/descargas.
- `install` corre el resolver SAT sobre las bases de sync: cierre de
  dependencias, orden de dependencias, requisitos `name=version` y `provides`
  sin versión.
- `query` (incluido `--orphans`), `search`, `info` y `files` leen las bases
  local y de sync.
- `history` enlaza cada transacción con la generación que produjo (`gen:NNNN`),
  `diff <generation>` compara la base viva contra el `packages.tsv` de una
  generación, y `rollback --last` reproduce la inversa de la última transacción
  exitosa desde la caché de paquetes (`--dry-run` soportado).
- El manejo de `.pacnew`/`.pacsave` sigue las entradas `backup` de `.PKGINFO`:
  los archivos de configuración modificados nunca se sobrescriben en silencio
  y sobreviven a las eliminaciones salvo `--nosave`.
- Los hooks ALPM estilo pacman (`/usr/share/libalpm/hooks`,
  `/etc/pacman.d/hooks`) corren alrededor de cada transacción, así que los
  hooks de generaciones de la distro funcionan también bajo xpm.

Ver [Usage](usage.md) para la referencia completa y [Architecture](architecture.md)
para los detalles de implementación.
