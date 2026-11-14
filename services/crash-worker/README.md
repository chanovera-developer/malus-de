# Malus DE Crash Reporter Worker

Worker de Cloudflare sin dependencias externas que recibe el `POST` JSON del botón **Enviar reporte** (`malus-de-crash-reporter`) y lo envía por correo con el `.txt` adjunto directamente a `malus-de@chano.dev`.

## Despliegue rápido en Cloudflare (`chano.dev`)

1. En el panel de Cloudflare de **`chano.dev`**:
   - Ve a **Email -> Email Routing -> Destination addresses** y verifica que `malus-de@chano.dev` aparece como **Verified** (con Email Routing habilitado para el dominio).
2. Despliega este Worker desde esta carpeta:
   ```bash
   cd services/crash-worker
   npx wrangler login
   npx wrangler deploy
   ```
3. Asocia el subdominio por defecto que usa Malus DE:
   - En **Workers & Pages -> `malus-de-crash-worker` -> Settings -> Domains & Routes -> Add -> Custom Domain**, añade **`crash.chano.dev`**.
   - El cliente de escritorio enviará automáticamente a `https://crash.chano.dev/report`.

> Si prefieres otra ruta o dominio (por ejemplo `https://chano.dev/api/malus-crash`), puedes configurarlo sin recompilar creando `~/.config/malus-de/crash-reporter.toml`:
> ```toml
> endpoint = "https://tu-ruta.chano.dev/report"
> ```
> o con la variable de entorno `MALUS_CRASH_ENDPOINT`.

## Prueba rápida con `curl`

```bash
curl -i -X POST https://crash.chano.dev/report \
  -H 'Content-Type: application/json' \
  -d '{"component":"malus-de-compositor","summary":"Prueba","filename":"test-crash.txt","report":"Linea 1\nLinea 2"}'
```

