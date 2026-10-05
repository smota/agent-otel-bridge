# M2 — diagnóstico somente leitura, tentativa 01

Estado: tentativa concluída; milestone M2 partial. Executor solicitado: gpt-6.1-sol high, subagente m2_diagnosis. Coordenador persiste evidência. Janela: 2026-09-30 22:20–22:26:31 +02:00; dez batches. Nenhuma alteração de runtime.

## Contrato

Investigar identidade/ausência do daemon, configuração por cliente e cobertura armazenada; distinguir fato, hipótese e teste restante. Sem restart, instalação, mudanças de hooks, execução de modelos ou carga. Não expor conteúdo de prompts, logs brutos sensíveis ou credenciais.

## Evidência recebida após três batches

- Binário bridge ativo, invocado apenas com --version: 0.5.2.
- Hashes ativos continuam divergentes de active.json; backups correspondem ao manifesto.
- Arquivos ativos modificados em 23/09; manifesto em 14/09. Timestamps não provam autoria, conteúdo da alteração ou forma de instalação.
- Nenhum processo bridge observado. Diretório logs/ canônico documentado está ausente.
- HKCU Run aponta para binário canônico com start.
- Endpoint OTLP nos escopos Process/User: http://127.0.0.1:4318; protocolo http/protobuf.
- Nenhum idle timeout explícito nos escopos consultados; source atual desabilita idle shutdown por padrão. Não presumir que o código fonte atual corresponde ao binário ativo 0.5.2.

## Hipóteses e limites

A ausência de processo impede o recebimento pelo daemon neste instante. O motivo de sua ausência permanece desconhecido. Divergência de hashes e timestamps define drift de instalação, não demonstra causa de parada. Startup registrado não prova startup executado.

Próximo: configurações decodificadas e consultas agregadas no ClickHouse; separar native/bridge e quatro clientes. M2 não concluído.

## Backend: janela de 24 horas, consulta às 22:23 +02

Relógio UTC do ClickHouse alinhado com Windows UTC na observação do executor.

| Serviço de traces | Contagem de spans | Último timestamp observado UTC | Spans com agent.hook.event |
|---|---:|---|---:|
| codex-app-server | 1985509 | 2026-09-30 20:22:56Z | 0 |
| codex-exec-server | 379 | 2026-09-30 20:20:26Z | 0 |

Nenhuma linha agent-otel-bridge no agregado da janela. É evidência delimitada à consulta e ao armazenamento, não ausência histórica universal. Números são spans, não tarefas, usuários ou chamadas LLM únicas.

Logs na janela: 8488 codex-app-server; 30801 sem service.name. Inventário de metadados de séries: codex-app-server 253912 linhas e 367 nomes de métricas distintos; codex-exec-server 231 linhas e 4 nomes. Metadados usam buckets de seis horas e não provam frescor das amostras; não interpretar volume de metadados como número de séries ativas sem deduplicação adequada.

Conclusão delimitada: ingestão nativa do Codex persiste sem marcas de hook enquanto o processo bridge não é observado. Isso explica uma via possível para o predomínio de Codex, sem provar ainda todas as causas por cliente.

Consulta histórica de 17 dias excedeu limites de leitura de 3M/10M linhas configurados pelo executor. Não houve conclusão sobre ausência histórica. Próximo recorte: dias específicos 14/09 e 23/09, sem ampliar indiscriminadamente a leitura.

## Fechamento da tentativa

- Binários ativos correspondem exatamente aos hashes de target/release do checkout. Nenhum dos nove diretórios versions/ corresponde aos hashes ativos. Isso não identifica o commit de origem da compilação.
- Contagem final de processo bridge: zero. Pipes agent-otel/agy-otel encontrados: zero.
- Boot Windows: 2026-09-30 14:47:42.5 +02. Registro Run permanece presente; execução no login não comprovada.
- Amostra dos cem eventos Application mais recentes de IDs 1000/1001 desde 23/09: nenhum matching bridge. Amostra limitada não exclui crash.
- Histórico stderr de reparo de 14/09 contém 22 spans transformed/accepted, quatro inválidos e zero shutdown drops; não explica ausência atual.
- Código de startup Windows descartou stdout/stderr e ignorou resultado de spawn no trecho inspecionado. Hook nativo tenta envio limitado e retorna zero; não chama spawn_daemon_detached. Logo, executar hook não garante recuperação de daemon nem entrega.
- Default de idle shutdown desabilitado também no commit declarado d206a0c; sem override nos escopos Process/User/Machine consultados. Hipótese de idle normal não sustentada; origem do build ativo continua desconhecida.
- Portas Windows 4317/4318 pertencem ao PID 6984 otelcol; listeners de encaminhamento 14318/8080 presentes. Ingestão nativa atual refuta indisponibilidade total do Collector/backend.
- Na última hora consultada, contagens igualaram identidades únicas (trace_id,span_id): Codex app 211729/211729; exec 80/80. Não detecta duplicação lógica com IDs distintos.

## Histórico 23/09 UTC

Entre 18:30:58.498707800 e 20:15:44.966422200 UTC: 2448 spans do bridge, todos antigravity; 618 PreInvocation, 617 PostInvocation, 598 PreToolUse, 595 PostToolUse, 20 Stop. Todos únicos por (trace_id,span_id), com atributo de conversa; três trace IDs. Nenhum parent_span_id ou gen_ai.tool.call.id.

Contexto: 997 fresh, 1138 stale, 313 missing; 339/598 PreToolUse com workspace. Não inferir perda ou quebra de contrato só da diferença de contagens ou destes estados. Interseção de traces Codex nativo/bridge naquele dia: zero, mas amostra bridge só AGY não é teste válido de correlação Codex.

Recorte 14/09 sem linhas: retenção não foi distinguida de ausência; resultado não prova que não houve emissão.

## Fichas dos quatro clientes

| Cliente | Configuração e evidência | Próximo teste delimitado |
|---|---|---|
| AGY | Cinco comandos codificados globais para path canônico, sem --client; histórico 23/09 atribuído antigravity. Identidade pode depender do payload; precedência de sessão não comprovada. | Uma sessão marcada com leitura inofensiva por superfície usada; comparar evento real, atribuição, contexto e parentage com lifecycle esperado. |
| Claude | Três comandos com --client claude e guarda GROK_WORKSPACE_ROOT; telemetria/enhanced habilitadas, conteúdo sensível desabilitado. Nenhuma linha Claude nos recortes inspecionados. | Confirmar exportadores/ambiente efetivos e uma leitura marcada; separar native/bridge. Testar supressão do comando importado no ambiente real Grok. |
| Codex | Três comandos --client codex; hooks=true, hashes trusted persistidos e trust do repo. Logs/traces/metrics HTTP protobuf em 4318. Native atual comprovado; bridge não. | Uma leitura marcada em CLI/Desktop usados; comprovar invocation, span recebido e comparar IDs/parentes, sem assumir propagação. |
| Grok | Três comandos próprios --client grok; importados Claude têm guarda; sem override selecionado no config global. Sem linha Grok nos recortes. | Confirmar próprios/importados e GROK_WORKSPACE_ROOT; uma leitura marcada, uma entrega por evento definido, atribuição e supressão corretas; aliases conflitantes em fixtures. |

Esses testes reais ainda não foram executados nesta tentativa e dependem de runtime identificável. Autenticação anterior não comprova cobertura.

## Consultas reproduzíveis

Executar somente SELECT com limites via `wsl -d Ubuntu --exec docker exec signoz-telemetrystore-clickhouse-0-0 clickhouse-client --query <SQL>`.

```sql
SELECT serviceName, count(), min(timestamp), max(timestamp),
       countIf(mapContains(attributes_string,'agent.hook.event'))
FROM signoz_traces.distributed_signoz_index_v3
WHERE timestamp >= now()-INTERVAL 24 HOUR
GROUP BY serviceName ORDER BY count() DESC LIMIT 30
SETTINGS max_execution_time=20, max_rows_to_read=3000000
FORMAT TSV
```

Histórico: restringir timestamp a [2026-09-23 00:00:00,2026-09-24 00:00:00) UTC e serviceName='agent-otel-bridge'. Agregar somente identidades, evento/agente, presença de atributos e contexto. Não ler bodies/prompts.

Métodos adicionais: hash de binários; --version; CIM de processos/serviço/boot; lista de pipes; ambiente selecionado; decodificação UTF-16LE sem execução; eventos Application limitados; leitura main.rs/start, IPC spawn/find_binary, daemon config/shutdown, cliente/transformation; git show do commit declarado. Sem novas chamadas LLM ou smoke de harness nesta tentativa.

## Aceite e próximo passo

Aceita a distinção native Codex versus bridge sem processo/pipe e drift de instalação. Não aceitos como concluídos: causa de saída, origem do build e cobertura real dos quatro clientes. M2 permanece partial; não promover automaticamente para done.

Próxima fatia de M2: desenhar e executar diagnóstico controlado de inicialização com stdout/stderr persistidos e ambiente declarado, preferindo isolamento do runtime ativo. Antes de qualquer efeito, salvar intenção, binário/hash, pipe/endpoint e critério de término. Não sobrescrever binários nem reinstalar para esconder a falha. Pesquisa M3 pode avançar independentemente com a linha de base M1.
