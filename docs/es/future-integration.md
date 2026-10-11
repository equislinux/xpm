# xpm — Notas de integración

Cómo encaja `xpm` en el flujo de la distribución, qué queda abierto antes de
que sea la vía de instalación principal y qué interfaces conviene mantener
estables.

---

## Dónde está xpm hoy

`xpm` está **conectado de extremo a extremo**:

- El resolver SAT corre dentro de `install` y `upgrade`: cierre de
  dependencias, orden de dependencias, requisitos `name=version` y `provides`
  sin versión.
- La instalación de `.xp` locales, la eliminación y las actualizaciones con
  dependencias funcionan sobre el mismo motor de transacciones.
- El endpoint nativo (`https://equislinux.github.io/x-repo/x/$arch`, separado
  del endpoint pacman en `/repo/x86_64`) está publicado y validado con
  `xpm sync/search/deps/install`.
- El sistema se sigue aprovisionando con **pacman + el repositorio firmado
  `[x]`**: `xpm` coexiste como canal nativo, no sustituye la vía de producción.

## Antes de que xpm sea la vía activa

Lista de producción pendiente (los elementos de recuperación y endurecimiento
cerraron antes):

1. **Resolver — ciclo de `gcc-libs`.** `Unsolvable(Conflict)` con paquetes que
   dependen de `gcc-libs` incluso en un root limpio.
2. **Extracción de hardlinks.** Falla al instalar paquetes reales de Arch con
   hardlinks (`zoneinfo` bajo `usr/share/zoneinfo/`).
3. **Hitos de producción** (Fase 8 y 9 de la ROADMAP del repo): tests de escala
   sobre repositorios grandes ya en suite; el benchmark directo contra pacman
   necesita root y queda manual. Restan: fuzzing, auditoría de errores
   (descargas parciales, paquetes corruptos, disco lleno) y objetivos post-v1.0
   (bindings Python, i18n, TUI, selección de mirrors, caché configurable).

## Integración con xpkg

`xpm` y `xpkg` son binarios complementarios que comparten el formato de paquete:

| Herramienta | Rol | Analogía |
|------|------|---------|
| `xpm` | Gestor de paquetes — instalar, eliminar, actualizar, resolver dependencias | `pacman` |
| `xpkg` | Constructor de paquetes — compilar, empaquetar, lint, gestionar repos | `makepkg` + `repo-add` + `namcap` |

Ciclo de vida: receta (XBUILD/PKGBUILD) → `xpkg build` → paquete `.xp` → base
del repositorio → hosting estático → `xpm sync` → `xpm install`/`upgrade`
(verificar firma, extraer, ejecutar `.INSTALL`, registrar localmente). xpkg
también puede emitir una base ALPM estándar; xpm lee tanto campos estándar
como extendidos de xpkg (`FILENAME`, `SHA256SUM`, `URL`), manteniendo
compatibilidad con repositorios Arch.

## Interfaces que conviene mantener estables

- **Formato de paquete**: `.xp` == estructura ALPM `.pkg.tar.zst` (tar.zst con
  `.PKGINFO`, `.BUILDINFO`, `.MTREE`, `.INSTALL` opcional). La versión del
  formato es implícita en `.PKGINFO`; ambas herramientas deben moverse en
  lockstep.
- **Layout del repositorio**: endpoint `<server>/<repo>.db` (+ `.files`), árbol
  nativo en `https://equislinux.github.io/x-repo/x/$arch`, firmas detached
  `.sig`, keyring `trustedkeys.gpg`.
- **Configuración**: `/etc/xpm.conf` (TOML), repos de usuario en `/etc/xpm.d/`,
  variables `$repo`/`$arch`, semántica de `sig_level`.
- **Base local**: `/var/lib/xpm/local/<pkg>/` con `version`, `reason`, `origin`,
  `files`, `depends` y `provides`; bases de sync en `/var/lib/xpm/sync/`.
- **Contrato de scriptlets**: funciones bash de `.INSTALL` con `XPM_ROOT_DIR`,
  `XPM_PKG_NAME`, `XPM_PKG_VERSION`.

## Ver también

- Roadmap del repo: `../../ROADMAP.md`
- Integración xpm/xpkg: [`../INTEGRATION.md`](../INTEGRATION.md)
- Arquitectura del resolver: [`../RESOLVER.md`](../RESOLVER.md)
