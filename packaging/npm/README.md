# llama-scale

Prebuilt binaries for [llama-scale](https://github.com/tokenring-ai/llama-scale), an OpenAI-compatible LLM router with session affinity and least-connections load balancing.

This npm package installs a `llama-scale` CLI that automatically selects the correct native binary for your platform.

## Supported platforms

| Platform | Architecture |
|----------|--------------|
| Linux    | x64, arm64   |
| macOS    | x64 (Intel), arm64 (Apple Silicon) |

## Install

```bash
npm install -g llama-scale
```

## Usage

Create a `config.yaml` (see [config.example.yaml](https://github.com/tokenring-ai/llama-scale/blob/main/config.example.yaml) in the main repo), then run:

```bash
llama-scale --config config.yaml
```

The config path can also be set via the `MODEL_ROUTER_CONFIG` environment variable.

## Configuration

See the [main project README](https://github.com/tokenring-ai/llama-scale#configuration) for full configuration options.

Minimal example:

```yaml
server:
  listen: "0.0.0.0:8080"
  api_keys: ["sk-router-dev-key-change-me"]

backends:
  - name: openai
    url: "https://api.openai.com/v1"
    api_key: "${OPENAI_API_KEY}"
```

Secrets can be referenced as `${ENV_VAR}` in the config file.

## Links

- [Source repository](https://github.com/tokenring-ai/llama-scale)
- [Issue tracker](https://github.com/tokenring-ai/llama-scale/issues)

## License

MIT