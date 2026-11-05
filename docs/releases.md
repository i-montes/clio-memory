# Cómo se publica una versión

Publicar una versión es empujar una etiqueta. No hay PR de sumas que fusionar, ni release en borrador que publicar, ni secretos que crear o renovar: el workflow `.github/workflows/release.yml` hace el resto.

## Publicar

1. Subir `version` en `Cargo.toml` (y `Cargo.lock`) en un PR normal y fusionarlo en `main`.
2. Desde `main`, ya actualizado:

   ```sh
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```

La etiqueta, sin la `v`, debe ser igual a `version` en `Cargo.toml`. Si no lo es, el workflow falla antes de compilar nada.

## Qué hace cada trabajo

El workflow corre al empujar una etiqueta `v*`. Cada trabajo espera al anterior, así que si uno falla, los siguientes no corren.

1. **`version`**: compara la etiqueta con la versión de `Cargo.toml`.
2. **`check`**: llama a `ci.yml` (`workflow_call`) sobre el commit etiquetado. Son las mismas verificaciones de cualquier PR: `fmt`, `clippy -D warnings`, `test --all-features` en Ubuntu y macOS, `cargo deny`, `cargo audit`, `gitleaks` y `shellcheck install.sh`. En las etiquetas, `gitleaks` corre el binario de la CLI (v8.30.1, fijado por sha256) sobre todo el historial y respeta `.gitleaksignore`, porque `gitleaks-action` no escanea nada en un push de etiqueta sin commits nuevos.
3. **`build`**: compila con las features `model2vec,granite`. Cada binario sube como artefacto con el nombre de su asset.

   | Target | Runner | Cómo |
   |---|---|---|
   | `x86_64-unknown-linux-gnu` | `ubuntu-latest` | `cargo zigbuild` contra glibc 2.28 (`<target>.2.28`) |
   | `aarch64-unknown-linux-gnu` | `ubuntu-latest` | `cargo zigbuild` contra glibc 2.28 (`<target>.2.28`) |
   | `aarch64-apple-darwin` | `macos-latest` | `cargo build` |
   | `x86_64-apple-darwin` | `macos-latest` | `cargo build` (compilación cruzada) |
4. **`smoke`**: corre cada binario donde se puede y comprueba que `--version` dice `clio-memory <versión de la etiqueta>`.
   - `x86_64-unknown-linux-gnu` en `ubuntu-latest`, `aarch64-unknown-linux-gnu` en `ubuntu-24.04-arm` y `aarch64-apple-darwin` en `macos-latest`.
   - `x86_64-apple-darwin` solo si `arch -x86_64 true` funciona en el runner; si no, se salta y el resumen del trabajo lo dice. En esa plataforma solo se prueba `--version`, porque bajo Rosetta `install.sh` elegiría el binario de arm64.
   - En Linux comprueba también que el símbolo `GLIBC_` más alto (`objdump -T`) no pase de `GLIBC_2.28`.
   - Hace una instalación de prueba con `install.sh` sobre los binarios recién construidos (`CLIO_MEMORY_DOWNLOAD_URL=file://…`, `CLIO_MEMORY_VERSION`, `CLIO_MEMORY_SKIP_ATTESTATION=1` y `CLIO_MEMORY_HAND_OVER=0`, que instala sin correr `setup`) y corre `~/.local/bin/clio-memory --version`.
5. **`publish`**: genera `SHA256SUMS`, firma la procedencia de los 4 binarios con `actions/attest-build-provenance` y crea el release con `gh release create "$TAG" … --generate-notes --verify-tag`, sin `--draft`. Es el único trabajo con `contents: write`, `id-token: write` y `attestations: write`; los demás solo leen. No usa secretos más allá del `GITHUB_TOKEN`.

Cada release lleva los 4 binarios (`clio-memory-x86_64-unknown-linux-gnu`, `clio-memory-aarch64-unknown-linux-gnu`, `clio-memory-aarch64-apple-darwin`, `clio-memory-x86_64-apple-darwin`) y `SHA256SUMS`.

## Si algo falla

- **Antes de `publish`:** no queda nada publicado. Se borra la etiqueta, se corrige y se etiqueta otra vez:

  ```sh
  git push --delete origin vX.Y.Z
  git tag -d vX.Y.Z
  ```

  Si la corrección es un commit nuevo, se vuelve a etiquetar ese commit con el mismo número.
- **En `publish`, antes de que exista el release** (falla la atestación o `gh`; `gh` borra su propio borrador si algo se corta): no queda nada publicado. Se borra la etiqueta y se vuelve a empujar la misma, como en el caso anterior.
- **En `publish`, después de crear el release:** el release es inmutable, así que no se corrige ni se borra. Se arregla con la siguiente versión (`vX.Y.Z+1`).

## Confianza en el binario, y su límite

- **Releases inmutables.** El repositorio debe tener activada la opción *immutable releases* de GitHub. Una vez publicado un release, ni sus assets ni su etiqueta se pueden cambiar ni borrar, así que el binario y las sumas de una versión quedan fijos. Para colar un binario falso habría que publicar una versión nueva, y eso exige escribir en el repositorio y pasar el workflow. GitHub, además, firma una atestación del release.
- **Sumas en el release.** `install.sh` baja el binario y `SHA256SUMS` de la misma versión y compara. Eso detecta un archivo dañado o cortado. Que el archivo sea legítimo lo garantiza la inmutabilidad, no la suma.
- **Atestaciones de procedencia.** El workflow firma cada binario con `actions/attest-build-provenance`, que lo liga al commit y al workflow que lo construyó. Si la máquina tiene `gh` con sesión, `install.sh` corre `gh attestation verify`: si la atestación no corresponde, la instalación se detiene; si `gh` no puede verificar (no está, no tiene sesión o no hay red), sigue con una línea en gris que lo dice.
- **El límite.** Sin un humano en medio, la confianza termina en la cuenta de GitHub de Iván y en el workflow. Quien controle eso puede publicar una versión. Las sumas escritas en el repositorio y fusionadas a mano habrían agregado una revisión humana, pero se eligió no tener ese paso.

## Configuración del repositorio de la que depende

### Immutable releases

Estado: **pendiente de activar por Iván.** Al 2026-11-05 el repositorio dice `enabled: false`.

Revisarlo (necesita permiso de administración):

```sh
gh api repos/i-montes/clio-memory/immutable-releases
# {"enabled":true,"enforced_by_owner":false}   <- activado
# {"enabled":false,...}                         <- pendiente
```

Activarlo (responde `204`):

```sh
gh api -X PUT repos/i-montes/clio-memory/immutable-releases
```

Desactivarlo sería `-X DELETE` sobre la misma ruta; no se debe hacer. Conviene activarlo **antes** de empujar la primera etiqueta: la inmutabilidad solo protege los releases publicados después.

### Permisos de Actions

El workflow declara sus permisos en cada trabajo (`contents: write`, `id-token: write` y `attestations: write` solo en `publish`), así que no necesita que el permiso por defecto del repositorio sea de escritura. Hoy es de solo lectura, y así debe seguir:

```sh
gh api repos/i-montes/clio-memory/actions/permissions/workflow
# {"default_workflow_permissions":"read","can_approve_pull_request_reviews":false}
```

Lo que sí hace falta es que Actions esté habilitado y permita las acciones que usa el workflow (todas fijadas por SHA de commit, con su versión en un comentario).

## Verificación a mano

Con los archivos del release descargados en una carpeta:

```sh
sha256sum -c SHA256SUMS --ignore-missing
gh attestation verify clio-memory-x86_64-unknown-linux-gnu --repo i-montes/clio-memory
```

`--ignore-missing` evita que falle por los binarios de otras plataformas que no se bajaron. En macOS, `shasum -a 256 -c --ignore-missing SHA256SUMS` hace lo mismo que `sha256sum -c --ignore-missing`.
