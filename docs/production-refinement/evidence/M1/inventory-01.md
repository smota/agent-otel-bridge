# M1 — inventário inicial, tentativa 01

Estado: completed, inventário aceito com limites abaixo. Início observado: 2026-09-30 22:13:19 +02:00.
Executor solicitado: gpt-6-luna medium, subagente m1_inventory. Coordenador persiste resultados; executor somente leitura. Sem instalação, restart ou alteração de hooks.

## Fonte e identidade

- Checkout: C:/Users/samue/code/agent-otel-bridge.
- Branch: docs/production-refinement-milestones.
- HEAD: a47a9ab2539fb878c1e521c6a30648c51f7fcf72; git describe: v0.6.0.
- Manifesto: C:/Users/samue/AppData/Local/agent-otel-bridge/active.json, declara 0.5.2-d206a0c-dirty.
- Hook esperado pelo manifesto: SHA-256 e4ebe144161c3515dcff5a915c100ca3cc75a4bf7839deb956e6080720d26d63; 152576 bytes.
- Bridge esperado: SHA-256 7c0d5d5b558ef6fe4574bc766d6bbb6a6e9ad720a4d284818e0c81d41d520f4e; 3558400 bytes.
- Hook real em bin/agent-hook.exe: SHA-256 E776F732E2773F7AEF1A70C86F8B6057824AA8070082B902585876690E05E030; 152576 bytes; difere do manifesto.
- Bridge real em bin/agent-otel-bridge.exe: SHA-256 24C73837129C674D80DF727B85D04915952FC9C73B9BFFB451E6A531497C6E9D; 3558400 bytes; difere do manifesto.
- O executor encontrou backups que correspondem aos hashes do manifesto. A origem dos binários ativos continua desconhecida.
- Oito templates JSON em contrib/dashboards/signoz; publicação ainda não verificada.

## Limitação da sondagem de estado

Inspeção pelo coordenador de crates/agent-otel-cli/src/local.rs, run_status: o código envia HealthPing com status-probe e imprime stopped em qualquer Err de try_send. Assim, o resultado histórico da sondagem não distingue processo ausente, pipe indisponível ou outro erro de envio. Conferir processo, caminho e pipe antes de atribuir causa.

## Processos e inicialização (batch 4 do executor)

- Consulta Win32_Process não encontrou agent-otel-bridge.exe ou agent-hook.exe no momento observado. Hook é processo transitório; sua ausência isolada é esperada.
- Serviço otelcol: Running, Auto, LocalSystem, PID 6984.
- Caminho configurado do serviço: C:/Program Files/OpenTelemetry Collector/otelcol.exe, com configuração C:/Program Files/OpenTelemetry Collector/config.yaml.
- Caminho/argumentos via processo não estavam visíveis por permissões; o caminho acima vem da configuração do serviço, não de verificação do arquivo carregado.
- Registro de inicialização Run, AgentOtelBridge: invoca bridge canônico com start. Registro de startup não comprova execução bem-sucedida.

## Próximas coletas

Hashes reais, versões, processos/serviços e inicialização; configuração efetiva dos quatro clientes; Collector Windows; implantação/versão/volumes WSL; inventário de dashboards via API autenticada sem expor credenciais.

## Collector e WSL (batches 5–7 do executor)

- Windows config.yaml: 2813 bytes, modificado em 2026-09-11 12:09; OTLP gRPC/HTTP recebe em 0.0.0.0:4317 e 0.0.0.0:4318.
- Pipelines traces, metrics e logs exportam por otlphttp/signoz para http://127.0.0.1:14318. Reader de métricas internas também aponta para 14318.
- Existem receivers hostmetrics e PostgreSQL; credenciais não foram registradas. Docker scrape é atribuído pela configuração a um collection-agent no WSL.
- Ubuntu e docker-desktop em execução, WSL2.
- Projeto Compose signoz, arquivo /home/sam/pours/deployment/compose.yaml, sete containers running observados.
- GET /api/v1/version: v0.141.1. GET /api/v1/health: ok. Isso comprova resposta do serviço, não cobertura dos quatro clientes.
- Imagens Signoz e ingester usam tag latest; registrar digest completo no fechamento. Versões observadas: ClickHouse/keeper 25.12.5, PostgreSQL 16, collector contrib 0.160.0.
- Quatro volumes persistentes: PostgreSQL, keeper, dados ClickHouse e scripts; caminhos sob /var/lib/docker/volumes. Nomes exatos pendentes do recibo final.
- GET /api/v2/dashboards respondeu; extração da estrutura de resposta ainda em verificação. Não concluir ausência de dashboards com base em campos null da primeira extração.
- Tentativa de obter versão por executável /signoz falhou por caminho inexistente; versão foi obtida via API.

## Limites

Dados recebidos do executor via recibo e mensagens; coordenador verificou independentemente API de dashboards e comandos codificados de Claude e AGY. Nenhum aceite de funcionamento do runtime ou causa raiz. Inventário M1 concluído; origem dos binários, precedência de sessão e importação original dos dashboards seguem para diagnóstico.

## Configuração dos clientes

Inspeção é dos arquivos globais; precedência efetiva por sessão será testada em M2.

| Cliente | Arquivo global | Registro observado |
|---|---|---|
| AGY | C:/Users/samue/.gemini/config/hooks.json | Seção agent-otel-bridge: PostInvocation, PostToolUse, PreInvocation, PreToolUse, Stop; PowerShell EncodedCommand |
| Claude | C:/Users/samue/.claude/settings.json | PreToolUse, PostToolUse, Stop; PowerShell EncodedCommand; RTK e herdr também presentes |
| Codex | C:/Users/samue/.codex/hooks.json | PreToolUse, PostToolUse, Stop; path canônico entre aspas |
| Grok | C:/Users/samue/.grok/hooks/agent-otel.json | PreToolUse, PostToolUse, Stop; PowerShell com path canônico |

Correção de coleta: busca literal do executor não encontrou bridge em Claude; o coordenador decodificou EncodedCommand em UTF-16LE e confirmou os três registros. Não registrar ausência por busca literal em comandos codificados. O teste booleano inicial de path em JSON do executor também foi inconclusivo por escaping; a verificação é sobre o comando interpretado.

Claude decodificado, para cada um dos três eventos: `if ($env:GROK_WORKSPACE_ROOT) { '{}' } else { & "C:\Users\samue\AppData\Local\agent-otel-bridge\bin\agent-hook.exe" <Event> --client claude }`. É configuração observada, não execução comprovada.

## Dashboards publicados

GET autenticado /api/v2/dashboards retornou 18 entradas. Credencial lida por nome SIGNOZ_API_KEY, nunca registrada. Os oito templates têm entradas correspondentes (source=user); correspondência de nome não comprova igualdade do JSON nem consultas corretas.

| Template | ID publicado |
|---|---|
| fleet-operations-fde | 01a09b4d-e1c2-7a00-8af1-e42f4cc0ab3b |
| developer-velocity | 01a091ef-5360-74d2-98d2-841f093849f6 |
| fleet-governance | 01a091ef-5319-7646-8fdd-efabd14f5708 |
| tokenomics-and-cost | 01a09381-6c85-798f-b036-3b3947bc3d82 |
| tool-archetypes-and-waste | 01a09381-6ccf-720c-b297-8c0a60b32d56 |
| agent-sre-loops | 01a091ef-533b-768a-917f-151c1c991f16 |
| turn-inspector | 01a091ee-025a-7219-9695-eee9131b561b |
| ai-agent-observability | 01a091c1-c5c7-7105-9963-436f893fa832 |

Demais dez registros, confirmados por GET independente do coordenador em http://localhost:8080/api/v2/dashboards:

| Nome publicado | ID |
|---|---|
| postgres-overview-6mzcn6bg | 01a08f63-47ed-7559-8b99-76e497f8742e |
| antigravity-cli-6gkb3b9s | 01a08f6b-fbeb-7439-a6a9-0809f9da72a0 |
| claude-code-metrics-dgyd3g77 | 01a08f6b-d576-7bb2-bc1c-40859a18ff08 |
| grok-nvkeqsj2 | 01a08f6b-a8d3-79b3-8e7f-c7994ee5773b |
| grok-build-grok-cli-uvo8geez | 01a08f6b-7e04-74ff-9c24-28d774057899 |
| codex-czfo4zim | 01a08f6b-558b-7cc2-81f7-f9777c731f04 |
| openai-h9imab6j | 01a08f6b-2c5c-715e-b891-ff98f19b4eee |
| claude-agent-sdk-my3xdign | 01a08f6b-0327-718d-98ad-6dce5b17fb3b |
| opentelemetry-collector-93r3tz48 | 01a08f64-f076-7d18-b147-56b1250e6c28 |
| host-metrics-8zwv70gc | 01a08f60-8429-7c41-bc0c-4a8f199645b6 |

AGY decodificado pelo coordenador: cada um dos cinco eventos chama `& "C:\Users\samue\AppData\Local\agent-otel-bridge\bin\agent-hook.exe" <Event>`. Não há argumento --client nesse comando; avaliar identificação no payload em M2.

## Imagens e volumes exatos

Digests observados pelo executor em Docker inspect; tags não substituem estes identificadores:

| Imagem | Digest |
|---|---|
| signoz/signoz:latest | sha256:c10fa03e103c76bba2d67452bd26925f08a69f2a7b8d0f7982cea0e4f81ce88e |
| signoz/signoz-otel-collector:latest | sha256:72aa1e4c1ec529f178e962c049be35cfd7abae02fe9a397edad10b4a9cba62fa |
| clickhouse/clickhouse-server:25.12.5 | sha256:cacf32d6884291dc2ff5e0156a97f46fc53ff7c929a7906d114e268a929dfd3a |
| postgres:16 | sha256:f1c3376c26f2609ab9f29f71f824103fe2fcd8ee0346485cb6122a4f93df6f94 |
| clickhouse/clickhouse-keeper:25.12.5 | sha256:525b8b0f93ccb371131c46f22a066ea43d33d74aba314203e8044609945d8524 |
| otel/opentelemetry-collector-contrib:0.160.0 | sha256:799dc6cf12c96192af37b5bdba804da8c10b3bc563b43cb90c3f3c58d9572ad6 |

Volumes: signoz-metastore-postgres-0-data, signoz-telemetrykeeper-0-data, signoz-telemetrystore-0-0-data, signoz-telemetrystore-user-scripts. Mountpoints: /var/lib/docker/volumes/<name>/_data.

## Reprodução e proveniência

Executor: git status/rev-parse/describe; Get-Content/Get-FileHash; CIM Win32_Process/Win32_Service; registro Run do usuário; parsing JSON de hooks; wsl --list --verbose; Docker ps/inspect/volume inspect/compose ls; GET /api/v1/version, /api/v1/health, /api/v2/dashboards. Coordenador: parsing dos hooks e decodificação Base64 UTF-16LE sem executá-los; GET autenticado de dashboards; leitura de local.rs.

Nenhum gerador foi identificado na descoberta delimitada; README documenta importação manual via UI/API. Isso não comprova inexistência de automação externa. Os oito templates serão comparados às definições publicadas em M5.
