# xpm — Notas de integración futura

Cómo se espera que `xpm` vuelva a entrar en el flujo de la distribución X, las condiciones
previas antes de que pueda ser el camino de instalación activo y los contratos (seams) que
deberían permanecer estables mientras tanto. Este documento registra la comprensión actual
tomada del ROADMAP a nivel de workspace, del README/ROADMAP de este repo y de
`docs/INTEGRATION.md`. Nada de esto es trabajo commiteado; es contexto de planificación.

---

## Dónde está xpm hoy

Dentro de la iniciativa *reboot*, el tooling xpm/xpkg está **pospuesto** (ROADMAP del workspace,
Fase 4). El payload de la distro se empaqueta hoy con un flujo de PKGBUILD + makepkg como el
paquete `x-scripts` y se publica a través del repositorio compatible con pacman, consumido en los
sistemas instalados con pacman. `xpm` es una base de código Rust funcional y bien testeada que
todavía no está conectada al camino activo del *reboot*.

La consecuencia honesta: la documentación del comportamiento de cara al usuario puede desviarse
de lo que la distro entrega hoy, porque la distro entrega el camino pacman, no xpm. Trata las
funciones de xpm como el camino nativo futuro salvo que el flujo *reboot* las adopte.

## Cuándo podría volver xpm a estar activo

Razones que traerían a xpm (y a su compañero xpkg) de vuelta al alcance, según el ROADMAP del
workspace:

- La distro necesita el **repositorio `.xp` nativo** como canal de distribución (dejar de
  consumir el layout pacman para los paquetes x).
- Al **camino de instalación nativo** le quedan piezas (instalación local de `.xp`, cierres de
  `upgrade`, rollback) para sustituir al solver de pacman y a la selección por nombre; el
  resolver SAT ya está conectado a `install`.
- El empaquetado reproducible y con lint (xpkg) con firmas OpenPGP se convierte en un requisito
  duro del pipeline de payload.

Hasta entonces, xpm no debe tratarse como dependencia del build de la distro, y el flujo *reboot*
no debe añadir suposiciones sobre xpm instalado.

## Condiciones previas antes de que xpm sea el camino activo

Huecos a nivel de código que deben resolverse cuando la herramienta se reactive (honesto, del
propio roadmap del repo y del `main.rs` actual):

1. **Resolver los cierres de upgrade.** `install` ya usa el resolver SAT sobre las bases
   sincronizadas (orden de dependencias, `name=version`, `provides` sin versión). `upgrade`
   sigue comparando versiones por paquete y no calcula cierre todavía.
2. **Completar el endurecimiento de transacciones.** El roadmap del repo lista pendientes:
   gestión de archivos de configuración `.pacnew`/`.pacsave`, ejecución de alpm-hooks más allá de
   los scriptlets de `.INSTALL`, test end-to-end de upgrade, y tests de resolución de conflictos
   y rollback.
3. **Cerrar los hitos de preparación para producción** (Fase 8 y Fase 9 del ROADMAP del repo):
   benchmarks frente a pacman, stress testing contra un repositorio completo, fuzzing, auditoría
   de manejo de errores (descargas parciales, paquetes corruptos, disco lleno) y objetivos
   post-v1.0 (bindings de Python, i18n, TUI, selección inteligente de mirrors, caché
   configurable).
4. **Reconciliar inconsistencias de configuración** como el default del keyring GPG
   (`config.rs` usa `/etc/pacman.d/gnupg/` mientras que la guía del README usa
   `/etc/xpm/gnupg/`).

## Integración con xpkg

`xpm` y `xpkg` son binarios complementarios que comparten el formato de paquete (según
`docs/INTEGRATION.md`):

| Tool | Rol | Analogía |
|------|-----|----------|
| `xpm` | Gestor de paquetes — instalar, eliminar, actualizar, resolver dependencias | `pacman` |
| `xpkg` | Constructor de paquetes — compilar, empaquetar, lint, gestionar repos | `makepkg` + `repo-add` + `namcap` |

Ciclo de vida: receta de fuente (XBUILD/PKGBUILD) -> `xpkg build` -> paquete `.xp` -> base de
datos de repositorio -> hosting estático -> `xpm sync` -> `xpm install`/`upgrade` (verificar
firma, extraer, ejecutar scriptlets de `.INSTALL`, registrar en local). xpkg también puede emitir
una base de datos estilo ALPM estándar; xpm lee tanto los campos estándar como los extendidos de
xpkg (`FILENAME`, `SHA256SUM`, `URL`), manteniendo compatibilidad con los repositorios de Arch.

El README del repo remite a `docs/INTEGRATION.md` para ver cómo trabajan juntas las
herramientas; mantén ese documento en sync cada vez que el formato cambie.

## Contratos (seams) que deberían permanecer estables

Como xpm puede estar inactivo durante un tiempo, estas interfaces deben tratarse como
superficies de compatibilidad y documentarse con cuidado cuando cambien:

- **Formato de paquete**: `.xp` == estructura ALPM `.pkg.tar.zst` (tar.zst con `.PKGINFO`,
  `.BUILDINFO`, `.MTREE` y `.INSTALL` opcional). La versión de formato es implícita en la
  estructura del `.PKGINFO`; ambas herramientas deben moverse en lockstep.
- **Layout de repositorio**: endpoint de sync `<server>/<repo>.db` (+ `.files`), árbol nativo
  xpm en `https://xlnux.github.io/x-repo/x/$arch` (no confundir con el endpoint pacman bajo
  `/repo/x86_64`), firmas como `.sig` separadas, keyring como `trustedkeys.gpg`.
- **Configuración**: `/etc/xpm.conf` (TOML), repos de usuario en `/etc/xpm.d/`, variables de URL
  `$repo`/`$arch`, semántica de `sig_level`.
- **Layout de la base de datos local**: `/var/lib/xpm/local/<pkg>/` con entradas `version`, `reason`, `origin`,
  `files`, `depends` y `provides`, DBs de sync bajo `/var/lib/xpm/sync/`.
- **Contrato de scriptlets**: funciones bash ejecutadas desde `.INSTALL` con las variables de
  entorno `XPM_ROOT_DIR`, `XPM_PKG_NAME` y `XPM_PKG_VERSION`.

## Convenciones de trabajo aplicables también en el futuro

Cuando se retome el trabajo en xpm, se aplican las convenciones del workspace igual que hoy: el
trabajo se hace en la rama `x/reboot` de este repo, los commits quedan locales (sin push), los
mensajes de commit no llevan emojis ni referencias a asistentes, y cada fase cierra con tests
locales antes de seguir. Registra cualquier decisión de integración en el repo o carpeta al que
pertenezca.

## Véase también

- Roadmap del repo: `../../ROADMAP.md`
- Roadmap a nivel de workspace (fases del *reboot*, incluida la fase pospuesta de tooling):
  `../../../ROADMAP.md`
- Integración xpm/xpkg: [`../INTEGRATION.md`](../INTEGRATION.md)
- Arquitectura del resolver: [`../RESOLVER.md`](../RESOLVER.md)
