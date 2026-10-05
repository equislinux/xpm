# xpm — Visión general y estado

Este documento describe qué es `xpm`, dónde se sitúa dentro del ecosistema X y su estado actual
y honesto dentro de la iniciativa *reboot*. Se basa en el contenido del repositorio tal y como
está hoy (README.md, ROADMAP.md, código fuente, tests, docs/), no en un estado futuro deseado.

---

## Qué es xpm

`xpm` es un gestor de paquetes moderno y de alto rendimiento escrito en Rust puro para la
distribución X. Está diseñado como un reemplazo nativo en Rust de `pacman` y `libalpm`:

- Formato de paquete nativo `.xp` (X Package, un archivo `tar.zst`) con metadatos `.PKGINFO`,
  `.BUILDINFO` y `.MTREE`.
- Compatibilidad con paquetes `.pkg.tar.zst` de Arch Linux y con las bases de datos de
  repositorio de estilo ALPM (`.db` / `.files`).
- Resolución de dependencias basada en SAT mediante la crate `resolvo`.
- Archivo de configuración TOML en `/etc/xpm.conf`.
- Verificación de firmas OpenPGP separadas (detached) con un `sig_level` configurable
  (`required` / `optional` / `never`).
- Gestión de repositorios con repositorios predefinidos más repositorios añadidos por el
  usuario en `/etc/xpm.d/`.

El proyecto pertenece a la organización `xlnux`. Los paquetes los construye la herramienta
compañera `xpkg` (repo `xlnux/xpkg`); `xpm` consume lo que `xpkg` produce.

## Posición en el workspace x-lnux

El workspace `/home/x0z/Documents/repos/x-lnux` no es un único repositorio; cada carpeta es un
repo independiente de GitHub con su propio origin. Las carpetas relevantes son:

| Carpeta  | Repo             | Rol |
|----------|------------------|-----|
| `x`      | `xlnux/x`        | La distro (build de imagen/ISO con archiso y aprovisionamiento) |
| `scripts`| `xlnux/scripts`  | Scripts/kickstart del sistema |
| `xpkg`   | `xlnux/xpkg`     | Herramienta Rust de empaquetado (builder) |
| `xpm`    | `xlnux/xpm`      | Gestor de paquetes en Rust (este repo) |
| `x-repo` | `xlnux/x-repo`   | Repositorio de paquetes y portal |

Este repo sigue las convenciones compartidas del workspace: el trabajo de la iniciativa *reboot*
vive en la rama `x/reboot` de cada repo, los commits quedan locales (sin push) y los mensajes no
llevan emojis ni referencias a asistentes.

## Estado actual (honesto)

El repositorio existe, compila y tiene una suite de tests considerable (tests unitarios en
ambas crates más tests de integración bajo `tests/`). Sin embargo, dentro de la iniciativa
*reboot*, `xpm` está **des-priorizado**:

- El ROADMAP a nivel de workspace marca la **Fase 4 "Tooling Rust (xpm/xpkg)" como pospuesta**
  (`POSPUESTA`). Por decisión del mantenedor, xpm/xpkg queda fuera del alcance actual.
- Hoy el payload de la distro se empaqueta como el paquete `x-scripts` mediante un flujo de
  **PKGBUILD + makepkg** y se publica en `x-repo` a través del repositorio **compatible con
  pacman**. El consumo en los sistemas instalados se hace con pacman, no con xpm.
- Por tanto, `xpm` **no es todavía el camino activo** en el flujo *reboot*. Es una base de
  código de tooling funcional con su propio roadmap interno, a la espera de retomarse cuando el
  repositorio `.xp` nativo se necesite de verdad (el resolver SAT ya está conectado a
  `install`).

El repo sigue publicando sus propios binarios como paquetes `.xp` en el árbol nativo de xpm
(ver el README para el bootstrap de claves y el checklist de firmas), que es independiente del
camino pacman usado hoy en producción. No hay que confundir ambos layouts: el endpoint nativo de
xpm es `https://xlnux.github.io/x-repo/x/$arch`, no el endpoint pacman bajo `/repo/x86_64`.

## Roadmap interno e hitos

`xpm` mantiene su propio ROADMAP.md dentro de este repo, con fases 0-10. Progreso destacado:

- Fases 0-1 completas: scaffolding, CLI con 8 subcomandos más `repo`/`usage`, config TOML.
- Fase 2 (puente FFI con libalpm) omitida a propósito: el proyecto fue directo a Rust nativo
  para evitar dependencias C.
- Fase 3 completa: resolver SAT con `resolvo`, comparación de versiones ALPM `vercmp`, parsing
  de dependencias, manejo de conflictos y tests de integración.
- Fase 4 completa: lectores de `.xp` / `.pkg.tar.zst`, parsers de metadatos, extracción de
  archivos y validación de integridad post-instalación.
- Fase 5 mayormente completa: parsing de `.db` / `.files` ALPM, base de datos local bajo
  `/var/lib/xpm/local/`, sync remoto con descargas HTTP, backend GitHub Pages y sustitución de
  variables de URL.
- Track de ejecución de seguridad (Fase 10): implementadas la verificación de `.db.sig` y
  `.sig` de paquetes y la carga de keyrings.

La convención de versionado del roadmap del repo es:

| Versión  | Hito |
|----------|------|
| `v0.1.0` | Fases 0-1 completas (CLI funcional con configuración) |
| `v0.2.0` | Fase 2 omitida (sin dependencia de libalpm) |
| `v0.5.0` | Fases 3-5 completas (motor nativo operativo) |
| `v0.8.0` | Fases 6-7 completas (seguridad + transacciones) |
| `v1.0.0` | Fase 8 completa (benchmarked, testeado, listo para producción) |

El `Cargo.toml` del workspace reporta actualmente la versión `0.1.0`.

## Sobre el estado de los comandos

No todos los subcomandos están conectados del todo con la lógica del motor. De
`crates/xpm/src/main.rs`:

- `sync`, `install`, `remove`, `upgrade` y `repo` despachan a lógica real de transacción y
  descarga.
- `install` ejecuta el resolver SAT sobre las bases sincronizadas: cierre de dependencias, orden
  de dependencias, requisitos `nombre=versión` y `provides` sin versión.
- `query` (incluido `--orphans`), `search`, `info` y `files` leen las bases local y sync; los
  huecos restantes antes del camino nativo son la instalación local de `.xp`, los cierres de
  `upgrade` y rollback.

Ver [Uso](usage.md) para la referencia completa y [Arquitectura](architecture.md) para los
detalles de implementación.
