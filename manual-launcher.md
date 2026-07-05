# Manual del Launcher SM64 Coop DX

## 1. Cómo encuentra el juego el launcher

El launcher resuelve la ruta del binario del juego con este orden de prioridad:

| Prioridad | Fuente | Ejemplo |
|-----------|--------|---------|
| 1 (más alta) | `--game-path` por CLI | `./sm64coopdx-launcher --game-path /ruta/al/sm64coopdx` |
| 2 | Variable de entorno `SM64COOPDX_PATH` | `export SM64COOPDX_PATH=/ruta/al/sm64coopdx` |
| 3 | Perfil activo → `profile.json` → campo `game_path` | `~/.local/share/sm64coopdx/profiles/<nombre>/profile.json` |
| 4 | `launcher.toml` → `[game].path` | `~/.config/sm64coopdx/launcher.toml` |
| 5 (default) | Búsqueda automática (ver abajo) | Busca en: `./sm64coopdx`, `../sm64coopdx`, `~/*sm64coopdx_Linux-*/sm64coopdx` |

Si un nivel falla (el archivo no existe), **pasa al siguiente** automáticamente.

### Búsqueda automática (nivel 5)

Cuando no hay ninguna configuración, el launcher prueba estos paths en orden:

1. `<directorio del launcher>/../games/mario64/sm64coopdx`
2. `<directorio del launcher>/sm64coopdx` (mismo directorio)
3. `<directorio del launcher>/../sm64coopdx` (directorio padre)
4. `~/*sm64coopdx_Linux-*/sm64coopdx` (release packages extraídos en home)

**En tu caso**, el launcher encuentra el juego automáticamente en:
```
~/sm64coopdx_Linux-1.5.1-autoUpdater/sm64coopdx
```

---

## 2. Estructura de archivos

### Archivos del launcher

| Ruta | Propósito |
|------|-----------|
| `~/.config/sm64coopdx/launcher.toml` | Configuración global (ruta del juego, ROM) |
| `~/.local/share/sm64coopdx/sm64config.txt` | Config del juego (mods, red, dynos) |
| `~/.local/share/sm64coopdx/mods/` | Mods instalados (`.lua`) |
| `~/.local/share/sm64coopdx/dynos/packs/` | DynOS packs |
| `~/.local/share/sm64coopdx/profiles/` | Perfiles de jugador |
| `~/.local/share/sm64coopdx/profiles/active.txt` | Nombre del perfil activo |
| `~/.local/share/sm64coopdx/profiles/<nombre>/profile.json` | Config por perfil |
| `~/.local/share/sm64coopdx/profiles/<nombre>/sm64config.txt` | Config del juego por perfil |
| `~/.local/share/sm64coopdx/profiles/<nombre>/saves/` | Directorio de partidas (legacy) |
| `~/.local/share/sm64coopdx/assets/` | Assets del launcher (sprites, sonidos, fuente) |
| `~/.local/share/sm64coopdx/baserom.us.z64` | ROM copiada automáticamente al lanzar |
| `~/.local/share/sm64coopdx/game_stderr.log` | Log de errores del juego |

### Archivos del juego (en `~/sm64coopdx_Linux-1.5.1-autoUpdater/`)

| Archivo/Directorio | Requerido | Propósito |
|---------------------|-----------|-----------|
| `sm64coopdx` | **Sí** | Binario del juego |
| `lang/English.ini` | **Sí** | Traducciones (validado por el launcher) |
| `dynos/` | Recomendado | DynOS resource packs base |
| `mods/` | No | Mods base del juego |
| `palettes/` | No | Paletas de colores |
| `Super Mario 64 (USA).z64` | **Sí** | ROM original (MD5: `20b854b239203baf6c961b850a4a51a2`) |
| `libdiscord_game_sdk.so` | No | Integración Discord |
| `coopdx_updater` | No | Auto-updater |

---

## 3. Cómo configurar

### Método recomendado: `launcher.toml`

Crear `~/.config/sm64coopdx/launcher.toml`:

```toml
[game]
# Ruta al binario del juego (obligatorio si no se detecta automáticamente)
path = "/home/mikky/sm64coopdx_Linux-1.5.1-autoUpdater/sm64coopdx"

# Ruta a la ROM (opcional — el launcher la busca automáticamente)
rom_path = "/home/mikky/sm64coopdx_Linux-1.5.1-autoUpdater/Super Mario 64 (USA).z64"
```

### Método alternativo: variable de entorno

```bash
export SM64COOPDX_PATH="/home/mikky/sm64coopdx_Linux-1.5.1-autoUpdater/sm64coopdx"
./sm64coopdx-launcher
```

### Método por perfil

En `~/.local/share/sm64coopdx/profiles/<nombre>/profile.json`:

```json
{
  "playername": "MiNombre",
  "game_path": "/ruta/personalizada/al/sm64coopdx",
  "skip_intro": true,
  "fullscreen": false,
  "no_discord": true,
  "skip_update_check": true,
  "headless": false
}
```

---

## 4. Qué pasa al presionar Enter (lanzar el juego)

```
1. Fade a negro (1.5 segundos)
2. VALIDACIÓN pre-lanzamiento:
   ├── ¿Existe {game_dir}/lang/English.ini?
   ├── ¿Es escribible ~/.local/share/sm64coopdx/?
   └── Si falla → error rojo en pantalla
3. BÚSQUEDA DE ROM:
   ├── Busca *.z64 con MD5 válido en:
   │   ├── Directorio del juego
   │   ├── ~/.local/share/sm64coopdx/
   │   ├── launcher.toml → [game].rom_path
   │   └── ~/*sm64coopdx_Linux-*/
   ├── Copia la ROM a ~/.local/share/sm64coopdx/baserom.us.z64
   └── Si no encuentra ROM → error rojo en pantalla
4. SPAWN del juego:
   ├── Working directory = directorio del binario
   ├── --savepath ~/.local/share/sm64coopdx/     (comparte filesystem con el launcher)
   ├── --configfile <perfil>/sm64config.txt       (config por perfil)
   ├── --enable-mod <mod> (mods activados en UI)
   ├── Argumentos de red (--server, --client, --coopnet)
   ├── Argumentos de perfil (--playername, --skip-intro, etc.)
   ├── stderr → ~/.local/share/sm64coopdx/game_stderr.log
   └── stdout → /dev/null
5. El launcher vuelve a la pantalla principal
   └── Detecta cuando el juego cierra y restaura el volumen de música
```

---

## 5. Diagnóstico de errores

### Si el juego no arranca

1. **Revisar el error en pantalla** — el launcher muestra errores en rojo bajo el logo
2. **Revisar logs**: `~/.local/share/sm64coopdx/game_stderr.log`
3. **Verificar la ROM**: MD5 debe ser `20b854b239203baf6c961b850a4a51a2`

### Errores comunes

| Error | Causa | Solución |
|-------|-------|----------|
| "Game binary not found" | No se encuentra el binario | Configurar `launcher.toml` o `SM64COOPDX_PATH` |
| "Language file not found" | Falta `lang/` junto al juego | El juego debe tener su carpeta `lang/` |
| "No valid SM64 US ROM found" | ROM ausente o MD5 incorrecto | Poner `baserom.us.z64` en el directorio del juego |
| El juego se congela en "Loading ROM Assets" | La ROM no se copió al savepath | Verificar permisos de `~/.local/share/sm64coopdx/` |

---

## 6. Mejoras futuras propuestas (basadas en el código fuente del juego)

### 6.1 Sincronización bidireccional de configuración

**Problema**: El launcher escribe `sm64config.txt` en `data_dir/` y el juego lee/escribe el del perfil (vía `--configfile`). Los cambios hechos en el juego (activar mods, cambiar red) no se reflejan en el launcher hasta reiniciar.

**Solución**: Al detectar que el juego cerró, re-leer `sm64config.txt` del perfil y sincronizar con el estado interno del launcher.

### 6.2 Soporte para el updater integrado

**Problema**: El juego incluye `coopdx_updater` para actualizaciones automáticas. El launcher no lo usa.

**Solución**: Botón "Update" en el menú que ejecute `coopdx_updater` y refresque la UI.

### 6.3 Aislamiento de partidas por perfil

**Problema**: Con `--savepath` apuntando al `data_dir` raíz, todos los perfiles comparten el mismo `sm64_save_file.bin`.

**Solución**: 
- Antes de lanzar, hacer backup del save file compartido
- Después de cerrar, guardar el save file en el directorio del perfil
- Al cambiar de perfil, restaurar su save file
- El juego tiene 4 slots internos; se podría mapear 1 slot = 1 perfil

### 6.4 Manejo del ROM setup screen

**Problema**: Si el juego no encuentra ROM, muestra `render_rom_setup_screen()` — un bucle infinito que espera drag & drop. El launcher no puede detectar este estado.

**Solución**: 
- El launcher ya valida y copia la ROM antes de spawnear, así que esto no debería ocurrir
- Como safety net: monitorear el proceso hijo; si no crea ventana en N segundos, matarlo y mostrar error

### 6.5 Hot-reload de mods

**Problema**: El juego carga mods al inicio (`mods_init()` en `main_game_init`). Cambiar mods requiere reiniciar.

**Solución**: El juego ya tiene `--enable-mod` por CLI. El launcher ya lo usa. Para hot-reload, se necesitaría modificar el juego (fuera del alcance del launcher).

### 6.6 Instalador de assets desde el launcher

**Problema**: Los assets del launcher (sprites, sonidos, fuente) deben copiarse manualmente a `~/.local/share/sm64coopdx/assets/`.

**Solución**: 
- Al primer inicio, detectar si los assets existen
- Si no, copiarlos desde un directorio `assets/` junto al binario
- O embeber los assets críticos en el binario (via `include_bytes!`)

### 6.7 Soporte para múltiples versiones del juego

**Problema**: Solo se puede configurar una ruta de juego. Si el usuario tiene varias versiones (stable, dev, custom), debe cambiar la config manualmente.

**Solución**: Campo `game_path` en `profile.json` ya lo soporta. Agregar un selector de versión en la UI de perfil.

### 6.8 Logging mejorado del juego

**Problema**: stdout se descarta, stderr va a un archivo. Si el juego imprime info útil por stdout, se pierde.

**Solución**: Redirigir tanto stdout como stderr a `game_output.log`, o hacer stdout → stderr con `Stdio::from(stderr_file)` para ambos.

### 6.9 Soporte para Discord SDK

**Problema**: El juego carga `libdiscord_game_sdk.so` si existe. El launcher limpia el entorno (`env_clear`), lo que podría romper la carga de bibliotecas dinámicas.

**Solución**: Verificar que `LD_LIBRARY_PATH` no sea necesario (el juego debería usar RPATH=`$ORIGIN`). Si falla, agregar `LD_LIBRARY_PATH` al whitelist del entorno.

### 6.10 Modo headless sin interfaz

**Problema**: El perfil tiene opción `headless`, pero el launcher siempre muestra UI.

**Solución**: Agregar flag `--headless` al launcher que salte la UI y lance el juego directamente con los args del último perfil/config.

---

## 7. Build y desarrollo

### Requisitos

- Rust 1.85+ (edition 2024)
- Dependencias del sistema: `libsdl2-dev`, `libsdl2-image-dev`, `libsdl2-mixer-dev`, `libsdl2-ttf-dev`

### Compilar

```bash
cd ~/sm64coopdx-launcher-rs
cargo build --release
# El binario queda en target/release/sm64coopdx-launcher
```

### Ejecutar tests

```bash
cargo test        # 122 tests
cargo clippy      # linting
```

### Instalar assets

```bash
mkdir -p ~/.local/share/sm64coopdx/assets
cp -r assets/* ~/.local/share/sm64coopdx/assets/
```

---

## 8. Referencia del código fuente del juego

El juego (`sm64coopdx`) tiene los siguientes puntos de entrada relevantes para el launcher:

| Archivo | Función | Relevancia |
|---------|---------|------------|
| `src/pc/pc_main.c:503` | `main()` | Punto de entrada, parsea CLI, inicia juego |
| `src/pc/pc_main.c:470` | `main_game_init()` | Carga assets, mods, dynos, audio |
| `src/pc/pc_main.c:544` | `main_rom_handler()` | Busca y valida ROM |
| `src/pc/rom_checker.cpp:96` | `main_rom_handler()` | Escanea paths por `*.z64` |
| `src/pc/loading.c:187` | `render_rom_setup_screen()` | Pantalla de "no ROM detected" (bucle infinito) |
| `src/pc/cliopts.c:59` | `parse_cli_opts()` | Parsea argumentos CLI |
| `src/pc/platform.c:299` | `sys_user_path()` | Path de datos del usuario (`SDL_GetPrefPath`) |
| `src/pc/platform.c:332` | `sys_resource_path()` | Path de recursos (directorio del exe) |
| `src/pc/fs/fs.c:45` | `fs_init()` | Inicializa filesystem virtual en el savepath |
| `src/pc/djui/djui_language.c:23` | `djui_language_init()` | Carga archivos de idioma desde `lang/` |
