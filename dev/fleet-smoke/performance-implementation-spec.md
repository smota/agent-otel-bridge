# Especificação executável de desempenho e entrega

Versão 1 — 2026-09-14. Estado: especificação para implementação candidata; não é evidência de código implementado. Base inspecionada: `66ba326` com alterações locais do laboratório. Revalidar diff e hashes antes de executar.

## 1. Escopo, precedência e entrega

Esta especificação fecha a proposta da [análise arquitetural](rust-performance-architecture-analysis.md). O [plano de execução](performance-implementation-plan.md) define responsáveis, dependências e gates. O manifesto `performance-work-items.json` contém a distribuição verificável de tarefas. Requisitos abaixo têm IDs estáveis para rastrear implementação e evidência.

A implementação foi autorizada e modifica somente fontes e binários candidatos; o manifesto acompanha seu estado, sem substituir evidências de aceitação. Não instalar, promover, reiniciar daemon ativo, alterar hooks globais, publicar ou executar uma frota paga como efeito de um teste unitário. A campanha de agentes permanece coordenada pelo Antigravity; especificação e implementação por subagentes Codex seguem a solicitação mais recente. Codex integra e revisa os candidatos.

Persistir código, fixtures sanitizadas, schemas e contratos em Git. Capturas, perfis, corpos OTLP e respostas dos agentes ficam em diretórios temporários de propriedade da campanha e são descartados depois da avaliação. Nenhuma dependência do laboratório entra na instalação do produto. Não criar scripts `.ps1`; controle portátil em Rust/Cargo e Python, com adapters nativos onde necessários.

### Requisitos invariantes

| ID | Obrigação | Evidência exigida |
|---|---|---|
| R01 | Hook fail-open, resposta JSON apropriada ao harness, exit 0, sem configuração em disco ou tracing no prompt | testes do processo real, watchdog, stdout e ambiente |
| R02 | Ownership de todo I/O pendente até conclusão; nenhuma espera implícita por timeout zero | testes de estado e de pipe nativo, revisão do unsafe |
| R03 | Limites para conexões, bytes, filas, tarefas e shutdown | saturação, ocupação e contabilidade de descarte |
| R04 | Consumidor de eventos não aguarda filesystem, Git, HTTP ou quotas | inspeção do grafo de chamadas e testes de dependência bloqueada |
| R05 | Contexto por workspace, sem atribuição cruzada ou credenciais | fixtures de dois projetos, worktree, mudança de branch e origem |
| R06 | OTLP interpreta aceitação, rejeição parcial, retry e resultado desconhecido | coletor de teste com respostas controladas |
| R07 | Contexto 0x04 e legado compatíveis; identidade e parentesco preservados | hook → IPC → daemon → OTLP, sem ambiente de tracing no daemon |
| R08 | SLAs não são relaxados nem aprovados por proxy | assertion por fronteira e todas as repetições |
| R09 | Relatório reconciliável com trace/span IDs e proveniência | schema, casos negativos e consulta MCP/API |
| R10 | Candidato isolado, sem mutação ativa e sem runtime persistente do laboratório | hashes antes/depois, cleanup dos recursos próprios |
| R11 | Retirar duração artificial da prova de desempenho; medir fronteiras explicitamente | relógio monotônico e origem da medição |

## 2. Contratos comuns

Os tipos abaixo são novos contratos de implementação, não APIs já disponíveis. Manter wrappers públicos atuais quando possível; consumidores internos migram para os tipos detalhados. Não remover aliases ou atributos existentes.

`SendOutcome = Completed { bytes } | Rejected { reason } | TimedOut { phase, completion_unknown }`. Sucesso significa escrita concluída, nunca exportação confirmada. Motivos têm enum finito: `invalid_size`, `connect_failed`, `pipe_busy`, `deadline`, `write_failed`, `partial_write`, `capacity`, `shutdown`. Código OS fica em diagnóstico, não em label de alta cardinalidade.

`IngressFrame` possui `msg_type`, `payload`, `received_at: Instant`, ordinal local e licença de bytes. `ResolvedEvent` possui input normalizado, contexto W3C resolvido, snapshot opcional e origem. `ExportBatch` possui spans, quantidade, tamanho Protobuf contabilizado, instante mais antigo e identidade local do lote. Nenhum tipo requer propagação de flags de tracing ao LLM.

`ExportOutcome = Accepted | PartiallyAccepted { rejected_count, warning } | Rejected { reason } | Unknown { reason }`. Contagens de rejeição fora de `0..=batch_count` tornam a resposta inválida. Resultado desconhecido não pode ser convertido em zero perdas. IDs de spans são criados uma vez antes de exportar e preservados em retry.

Frames inválidos são rejeitados, não transformados em `AgentHookInput::default()`. Preservar a semântica documentada de payload vazio quando permitida pelo tipo de mensagem. Eventos de controle não geram spans de ferramenta falsos. Payload explícito continua precedendo traceparent transportado e fallback de conversation ID. Nunca consultar `TRACEPARENT` ambiente do daemon para ligar eventos recebidos.

## 3. Ingresso e recursos limitados

Valores abaixo são defaults propostos, não benchmarks. Entram em `PipelineLimits`, carregado uma vez no daemon. A primeira implementação usa defaults fixos exceto opções de batch já existentes; testes injetam limites menores. Evitar adicionar dezenas de flags sem necessidade. Configuração inválida retorna erro no startup do daemon, nunca pânico dentro do hook.

| Recurso | Default proposto | Comportamento no limite |
|---|---:|---|
| Payload por frame | 16 MiB, preservando teto existente | rejeitar antes de alocar |
| Conexões em leitura simultânea | 32 | recusar/fechar novas conexões sem criar task leitora |
| Janela para header + payload | 100 ms absolutos desde aceitação | fechar conexão; contar `read_deadline` |
| Bytes de payload reservados, incluindo leitores e fila | 32 MiB | não esperar memória: rejeitar frame `capacity` |
| Fila de ingresso | 4.096 itens | admissão não bloqueante, descartar novo frame completo |
| Transformadores | 1 | serial por admissão, sem task por evento |
| Batch | 50 spans ou 1 MiB Protobuf ou 200 ms de idade | fechar no primeiro limite atingido |
| Fila de exportação de traces | 64 lotes e 16 MiB Protobuf | descartar lote novo, contar spans |
| Exportação simultânea de traces | 1 | backpressure somente na fila de exportação |
| Fila de métricas/quotas | 1 snapshot, até 1 MiB | substituir snapshot antigo, sem duplicar workers |
| Exportação de métricas | 1 worker separado | não ocupar o worker de traces |
| Drenagem no shutdown | 5 s globais | descartar resíduos com razão e cancelar tasks próprias |

Reservar bytes depois de validar o header e antes de alocar payload. Licenças RAII atravessam a fila e são liberadas somente após consumo/descarte. Uma conexão sem header usa apenas a licença de conexão e buffer fixo. O limite de tamanho legado permanece 16 MiB: reduzir para 256 KiB sem migração quebraria emissores externos.

O transformer libera o payload depois de construir o span e usa orçamento separado para estruturas expandidas. Aplicar máximo de 1 MiB Protobuf por span; span excedente é rejeitado com `span_size`, nunca truncado silenciosamente. Manter original input até validação, com no máximo um evento em transformação. Reservar buffers e contabilizar capacity de strings/vetores relevantes; não apresentar 32+16 MiB como teto de RSS, porque allocator, JSON expandido, recursos, stacks e runtime também consomem memória. Testar RSS estabilizado sob carga e reportar pico separadamente. Limitar JSON à profundidade padrão do parser, sem parsing ilimitado.

Impor durante a deserialização no máximo 65.536 nós JSON dinâmicos e 16 MiB agregados de conteúdo de strings/containers; exceder retorna `event_complexity`. Não materializar um Value ilimitado para depois contar. Conservar licença de ingresso até liberar payload/input. Os novos limites de complexidade e span são diferenças de admissão documentadas: o teto de frame legado não garante que todo evento de 16 MiB seja exportável.

Bytes de exportação significam `ExportTraceServiceRequest::encoded_len()` completo, incluindo resource/scope e overhead Protobuf. A licença de 16 MiB acompanha lote em fila e request em voo até resultado final; dequeue não a libera. Extras permitidos: um batch aberto <=1 MiB, um buffer de encode trace <=1 MiB, um snapshot metrics <=1 MiB e um encode metrics <=1 MiB. Rejeitar resource/config que sozinho exceda o teto de request. Um span que cabe isoladamente mas não cabe com resource/scope em request vazio é rejeitado por tamanho. Não acumular múltiplas cópias por retry.

O aceitação de pipe deve despachar a conexão já estabelecida mesmo se a criação da próxima instância falhar. Retentativas de criação usam backoff limitado e cancelável; não abandonar conexão conectada num loop sem leitor. Tasks leitoras são rastreadas em `JoinSet`, com cancelamento explícito no shutdown. Se uma leitura parcial for cancelada, encerrar essa conexão; não reiniciar `read_exact` no mesmo frame.

Fila de controle de capacidade 8 separada do tráfego de hooks; Shutdown cancela admissão diretamente. HealthPing e QuotaPing não aguardam HTTP. QuotaPing solicita refresh coalescido; não dispara N consultas simultâneas. A compatibilidade de funções públicas `run_server` é mantida por wrapper, enquanto o daemon usa API com limites e estatísticas explícitos.

## 4. Contexto puro e snapshots

Adicionar `build_span_from_resolved(...)` sem leitura de ambiente, filesystem ou relógio. Recebe timestamps, IDs/contexto e identidade resolvida. Manter builders anteriores como wrappers de compatibilidade, mas o daemon e o benchmark puro não os usam. Aplicar o mesmo padrão a enrichment: normalização de campos e classificação de ferramenta são puras; resolver workspace/identidade pertence ao daemon.

`ResolvedEvent` deve conter também `user_email` e `terminal_type` finais/opcionais, além de workspace/VCS/contexto W3C. O builder não procura fallback oculto quando esses campos são None. Testes de equivalência dos wrappers antigos usam ambiente controlado; novos testes de pureza injetam resolver que falha se houver acesso. Valores ausentes em emissores externos não são preenchidos com identidade arbitrária do processo daemon.

Remover a chamada a `collect_git_stats` do evento **Stop**. A primeira implementação preserva estatísticas fornecidas pelo harness e omite estatísticas ausentes; não fabricar zero, não executar Git em outro worker para contornar a proibição. Manter constantes e wrappers públicos necessários, fora do grafo de processamento. Não acrescentar libgit2 nesta rodada. Documentar a diferença observável: ausência passa a representar dado não fornecido.

Resolver workspace em ordem: `workspace_path` absoluto explícito; primeiro `workspace_paths` absoluto. Caminho relativo exige base explícita do evento; se ausente, registrar missing, sem resolver contra cwd global. Evento sem workspace não recebe contexto do diretório de startup do daemon. `project_root` fornecido não autoriza misturar campos derivados de outro workspace. Comportamento de cwd ambiente permanece somente nos wrappers legados, fora do caminho do daemon.

Cache de no máximo 128 entradas e 4 MiB de snapshots, política LRU. Lookup usa chave lexical absoluta e não faz canonicalize/stat. Canonicalização e resolução de `.git`, worktree/commondir ocorrem no worker; aliases são limitados pelo mesmo orçamento. Não converter genericamente caminhos Windows para lowercase: preservar semântica de diretórios case-sensitive. Identidade inclui caminho de worktree; dois worktrees do mesmo repositório não compartilham branch.

Snapshot fresco por 1 s; depois pode servir stale por até 5 s enquanto solicita atualização. Acima de 5 s, omitir campos derivados até novo resultado. Miss não bloqueia o evento: emitir fatos fornecidos, marcar `missing` e agendar refresh. Resultado negativo tem TTL de 1 s. Fila de refresh de 32 chaves deduplicadas, dois workers fixos. Snapshot >64 KiB é rejeitado; cache de identidade local no startup não substitui identidade explícita do emissor.

Worker faz somente reads/stat diretos, com leituras limitadas a 64 KiB por arquivo e 15 níveis ancestrais; configurações maiores resultam em metadata unavailable, não parse de trecho truncado como completo. Budget de refresh de 20 ms é cooperativo entre syscalls, não timeout garantido de syscall. Um filesystem bloqueado pode ocupar os dois workers: não criar substitutos ilimitados. Evento continua sem contexto; registrar workers ocupados. Threads de filesystem não são aguardadas indefinidamente no shutdown e mantêm ownership de sua memória até retornar.

Depois de 250 ms sem término, marcar worker como stuck. Com dois stuck, abrir circuito de refresh, não aceitar novas chaves e contar `circuit_open`; não criar substitutos. Um worker que retorna fecha sua condição stuck e permite trabalho novamente. A fila continua limitada mesmo durante bloqueio. No shutdown, liberar join handles depois do orçamento; dados compartilhados permanecem em Arc até retorno/thread exit. Não alegar cancelamento de syscall nem zero threads vivas antes do término do processo nesse cenário.

Campos novos propostos: `agent.context.state` (`provided`, `fresh`, `stale`, `missing`), `agent.context.age_ms` e `agent.context.source` (`event`, `workspace_cache`, `none`). Idade omitida para `provided`/`missing`; nunca misturar labels de métrica com caminhos/email. Sanitizar URLs antes de publicar snapshot; credenciais não entram no cache observável. Campos explícitos prevalecem sobre cache e ausência não equivale a `vcs.system=none`: somente probe concluído pode dizer que não há Git.

## 5. Batch, exportação e encerramento

`SpanBatcher` torna-se componente síncrono sem referência ao exporter: `push` e `take_due(now)` retornam batches. Timer mede idade do span mais antigo, não reinicia indefinidamente com chegada de eventos. Remover `biased` desnecessário; se usado, shutdown precede recepção. Processar no máximo 64 mensagens antes de conferir deadlines/controle. HTTP e coleta de quotas executam em tasks independentes rastreadas.

Exporter mantém reuso de conexão e Protobuf. Conectar até 2 s; cada request até 5 s; deadline total do lote 6 s incluindo retries e leitura da resposta; máximo três tentativas. Delays base 100 e 200 ms com jitter uniforme de 0 até o valor base. `Retry-After` válido prevalece; se ultrapassar o deadline, terminar sem dormir além dele. Relógio e RNG são injetáveis em testes, sem sleeps reais para verificar política.

Retry somente 429, 502, 503, 504 e falhas transitórias de transporte elegíveis; não repetir 400/401/403/500 por regra genérica. Uma resposta 200 deve ser decodificada como `ExportTraceServiceResponse`/metrics equivalente. Corpo protobuf vazio válido representa mensagem default; corpo inválido é erro de protocolo. Sucesso parcial não é repetido. Limite de resposta 4 MiB após descompressão, leitura incremental limitada; nunca `bytes()` sem bound. Honrar o content type e não tratar 2xx arbitrário como aceite pleno OTLP. Fontes: [OTLP](https://opentelemetry.io/docs/specs/otlp/).

Depois de falha final, não recolocar lote no fim da fila para repetir indefinidamente. Registrar contagem descartada ou unknown conforme a etapa, e manter o payload somente até o fim da tentativa de exportação. HTTP aceito não comprova visibilidade no SigNoz. Repetir após resposta perdida pode duplicar spans; conservar IDs e reportar duplicatas na validação.

HTTP 200 com protobuf inválido, content type incorreto, resposta excedente ou rejected_count maior que enviado resulta em `Unknown(protocol)`, sem retry: o backend pode ter aceitado antes de responder incorretamente. Parcial válido contabiliza aceitos = enviados - rejeitados e rejeitados exatos, sem inferir IDs individuais. Falha de conexão comprovadamente anterior ao envio é descarte por entrega inviável; perda de resposta depois de envio é unknown. Testes não confundem esses casos.

Shutdown cancela admissão e quotas, encerra leitores, drena ingressos e batch, fecha produtores de exportação e tenta exportar até deadline global de 5 s. Reduzir os timeouts de requests ao restante desse deadline. Ao expirar, contar filas/batch em memória como descartados e request já enviado sem resposta como unknown; não contar ambos como perda confirmada. Emitir um resumo interno bounded disponível ao harness de teste; exportação desse resumo é melhor esforço e não pode impedir encerramento.

## 6. Telemetria de diagnóstico e reconciliação

Novas métricas propostas para `docs/TELEMETRY_DICTIONARY.md`: `agent.bridge.events.total` por `stage` e `outcome`; `agent.bridge.dropped.total` por motivo finito; `agent.bridge.queue.items`, `agent.bridge.queue.bytes` por fila; `agent.bridge.context.refresh.total` por resultado; `agent.bridge.export.attempts.total` por resultado. Unidades, monotonicidade e escopo de processo documentados. Exportar contadores cumulativos para evitar perda de contagem por snapshot sobrescrito. Não instrumentar a própria exportação com novos spans recursivos.

Stages: `frame_received`, `admitted`, `transformed`, `export_queued`, `backend_accepted`. Um frame só é received depois da leitura completa validada. Contagens de conexão são separadas. Em cada fronteira: entradas = saídas + descartes + ainda em voo; retries não incrementam eventos únicos. Para sucesso parcial com contagem, IDs individualmente aceitos podem ser desconhecidos: não inventar a lista a partir da quantidade.

O teste mantém manifesto de eventos tentados, IDs esperados e evidência por etapa. Diagnóstico detalhado por evento existe apenas no candidato/laboratório, sem prompts ou corpos privados. No produto, contadores são fatos; thresholds e alertas ficam no backend.

## 7. Critérios de medição

R08 conserva os limites estritos: parser e variantes históricas >50.000 spans/s; IPC p99 <3.000 µs; harvester real p99 <150 µs; hook interno <1.000 µs. Tamanho reporta os dois julgamentos `<300000` bytes e `<307200` bytes; aprovação conservadora exige o primeiro até normalização explícita da ambiguidade KB/KiB. Não usar o guardrail mais permissivo como aprovação.

Separar JSON-only, builder puro, parse+builder puro, caminho histórico com fallback, legado completo, 0x04 completo, snapshot hit e harvest real. Snapshot hit não satisfaz a assertion de harvester. As assertions históricas continuam presentes após otimização para evitar trocar o benchmark e esconder regressão. Equivalência funcional: campos esperados, IDs e aliases, não apenas contagem de spans.

Warmup 2.000 e amostras 50.000 para parser, 100/2.000 para contexto; três repetições planejadas por candidato, ordem A/B/B/A para contrastes pareados quando aplicável, seed registrada. IPC 20/1.000 por envelope. Reportar p50/p95/p99/max e contagens válidas; amostra vazia, NaN, erro de identidade ou timeout impedem PASS. Repetições reprovadas permanecem no relatório.

Adicionar medição interna do hook conforme contrato IPC/hook complementar abaixo. Duração externa inclui criação de processo/stdin/exit e aparece separadamente. Spans atuais de duração aproximada não validam esse SLA. Linux/macOS nativos permanecem `not_measured` neste host; testes portáveis em CI não certificam desempenho nativo.

## 8. Relatório de campanha v1

Schema novo independente do `report-v2.schema.json` da frota: `performance-report-v1.schema.json`. Não renumerar o relatório fleet. Campos legados podem permanecer em `raw_reports`; `assertions` normalizadas tornam-se a autoridade do veredito.

Campos obrigatórios: `schema_version=1`, `spec_version`, `campaign_id`, `run_id`, `attempt_index`, `mode`, `started_at_utc`, `ended_at_utc`, `seed`, `environment`, `candidate`, `active_before`, `active_after`, `commands`, `assertions`, `trace_evidence`, `cleanup`, `verdict`. `candidate` inclui commit, dirty/diff digest, arquivos untracked relevantes e SHA256, toolchain/target/profile, hash/size de cada binário. Informações indisponíveis são null com motivo, não string fabricada. Segredos e command env completo são proibidos.

Cada assertion: `id`, `requirement_id`, `implementation_refs`, `test_id`, `boundary`, `unit`, `operator`, `threshold`, `samples`, `observed`, `status` (`passed`, `failed`, `not_measured`), `reason`, `evidence_refs`. `observed` pode ser null somente com razão. Registrar erro de processo separadamente de violação numérica.

`trace_evidence` contém trace/root/span/parent IDs esperados e observados, missing, duplicate, orphan, origem (`native`/`lab`), UTC window em epoch ms, backend identity sanitizada, ferramenta de consulta e status (`complete`, `partial`, `query_failed`, `not_checked`). Um hash Git curto não é trace ID. Consulta externa complementar não reescreve retroativamente o relatório bruto.

Veredito: failed se qualquer assertion obrigatória falhar; senão not_measured se alguma obrigatória não foi medida; passed somente se todas passaram. `scope=windows_candidate` explicita que não certifica outras plataformas. As entradas Linux/macOS são informativas fora do escopo e não silenciosamente removidas. Local collector e backend visibility são assertions distintas; campanha determinística local não pode declarar SigNoz validado.

CLI implementada: `python dev/fleet-smoke/perf_campaign.py --suite regression --attempt-index 1 --seed 42 --repeats 3`. Para campanhas repetidas, usar `perf_loop.py` e seu ledger, conforme o README. `--suite` aceita `regression`, `faults`, `performance`, `confirmation`; índices 1..5. Schema inválido, binário ausente ou consulta falha jamais vira aprovado. Contrato de códigos: 0 passed, 1 falha de assertion, 2 entrada/configuração inválida, 3 evidência obrigatória ausente; preservar wrappers atuais se dependem de nonzero genérico. A presença da CLI não significa cobertura completa: consultar [cobertura de aceitação](performance-acceptance-coverage.md).

O controlador `perf_loop.py` gera `campaign_id` uma vez (UUID aleatório), passa `--campaign-id` para todas as tentativas e mantém o ledger. Invocação standalone sem ID gera uma campanha de uma tentativa e registra esse modo; não permite escolher índice >1 sem ledger fornecido pelo controlador. `run_id`, trace ID e root span ID são novos por tentativa. `--repeats` significa somente `measurement_repetitions`; a execução fleet usa `--repeat 1` por cenário. Não executar agentes pagos pelo comando de performance.

O schema tem duas variantes discriminadas por `record_type`: `attempt` contém os campos acima; `campaign` contém `campaign_kind`, `campaign_id`, `max_attempts=5`, `attempts_consumed`, `state_history`, referências aos relatórios de tentativa, `coverage`, `decision`, `terminal_reason` e `verdict`. Cada transição tem estado anterior/novo, timestamp e motivo. O validador verifica índices contíguos sem duplicatas, identidade estável de campanha, IDs de run distintos e limite de cinco. Um parâmetro CLI isolado não é prova do limite global.

## 9. Matriz de aceitação

| Test ID | Requisitos | Caso e oráculo |
|---|---|---|
| A01 | R01,R02 | stdin vazio, truncado, EOF atrasado, pipe ausente/ocupado: saída JSON e exit 0; ownership correto |
| A02 | R02 | conclusão antes/depois de cancel, cancel NOT_FOUND, budget sub-ms, escrita parcial: estados e bytes corretos |
| A03 | R03 | header inválido, 16 MiB+1, leitura parada, fila/bytes/conexões cheios: limites nunca excedidos e razões contadas |
| A04 | R04,R05 | filesystem/quotas/exporter bloqueados: ingresso continua ou rejeita por capacidade explícita, sem bloquear consumidor |
| A05 | R05 | dois workspaces, worktrees, branch alterada, relativo sem base, cache expirado, origem com token: nenhum cruzamento/vazamento |
| A06 | R06 | 200 completo/parcial/malformado, 429+Retry-After, 500, 503, timeout e resposta perdida: política e contagens exatas |
| A07 | R03,R06 | shutdown sob rajada e request em voo: deadline global, resíduos classificados e recursos próprios encerrados |
| A08 | R07 | precedência explícita→0x04→conversation, daemon com ambiente conflitante: trace/parent esperados |
| A09 | R08,R11 | decomposição, equivalência, cache hit/miss, cronômetro interno/externo distintos: limites estritos e proveniência |
| A10 | R09 | remover/trocar ID, duplicar span, falsificar success count, NaN ou schema: oráculo reprova independente do runner |
| A11 | R10 | comparar ativo antes/depois, timeout/cancel no harness: nenhuma mutação; zero recursos próprios remanescentes |
| A12 | R07,R09 | trace longo com progresso e root final: relações nativas verificadas via API/MCP, synthetic não substitui native |

Testes de aceitação são implementados junto de cada alteração e confrontam efeitos, não reproduzem apenas a estrutura da função. Nenhuma task declara concluído com teste filtrado que executou zero casos. O plano lista comandos existentes e alvos novos separadamente.

Precisões obrigatórias para os testes:

- A06/HTTP 500: exatamente uma request, resultado final rejeitado, sem retry. HTTP 200 parcial: quantidade rejeitada exata e nenhuma segunda request.
- A08: o envelope 0x04 carrega o `TRACEPARENT` do processo hook. Essa é a representação de ambiente herdado no transporte, não uma nova precedência. Conflito entre payload, ambiente do filho e ambiente do daemon deve selecionar payload; sem payload seleciona ambiente do filho transportado; ambiente do daemon nunca participa.
- A04: com exporter bloqueado por barreira controlada e refresh/quotas também bloqueados, enviar 24 eventos de 1 KiB em quatro produtores; exigir 24 frames transformados em até 1 s no candidato release isolado, sem liberação das barreiras. Teste de saturação usa limites injetados de dois itens/4 KiB, 100 frames e exige cada frame admitido ou rejeitado explicitamente, sem exceder licenças e com dois workers de refresh no máximo. O primeiro é teste funcional com deadline, não SLA de latência; a medição IPC continua independente.
- A12: a espera/loopback real de 30 s do cenário longo é controle de observação. Tem span próprio de laboratório; seu tempo é excluído de todos os samples de performance e da atribuição de duração do modelo. Não remover esse controle ao retirar a duração aproximada do daemon das provas de timing.

## 10. Decisão fechada de I/O pendente e finalização do hook

### Ownership e API

Implementar `attempt_send_until(endpoint, msg_type, payload, absolute_deadline) -> SendAttempt`, onde `SendAttempt` é `Complete(Result<usize, SendError>)` ou `CleanupRequired { error, pending: PendingWrite }`. `PendingWrite` possui uma operação alocada em Box estável contendo handles de pipe/evento, frame próprio e `OVERLAPPED`. Submeter WriteFile somente depois de fixar o endereço. Não mover a estrutura interna após submissão; mover o Box/token é permitido. Campos internos privados e construção insegura restrita ao módulo de transporte.

Em conclusão imediata ou assíncrona, consultar bytes transferidos e exigir frame inteiro. Em timeout ou falha de espera com conclusão incerta, solicitar `CancelIoEx` para aquele OVERLAPPED e devolver o owner. `ERROR_NOT_FOUND` no cancelamento não prova término: observar a conclusão mesmo assim. Não liberar buffer, estrutura ou eventos enquanto o kernel ainda pode referenciá-los. Fontes: [CancelIoEx](https://learn.microsoft.com/en-us/windows/win32/fileio/cancelioex-func), [WriteFile](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-writefile).

`PendingWrite::drain(self)` espera conclusão terminal com `GetOverlappedResult(..., TRUE)` antes de liberar recursos. Drop implementa a mesma drenagem como backstop para uso incorreto do token. Handles/evento permanecem privados e válidos, associados a uma única operação, sem reset/close externo. Nessas condições, retorno da espera observa conclusão: classificar sucesso, cancelamento ou outro erro terminal e liberar uma vez. Estado injetado que ainda indica pendência continua aguardando; nunca retornar de Drop liberando esse estado. Violação de invariantes de handles é defeito, não timeout recuperável. [Semântica da espera](https://learn.microsoft.com/en-us/windows/win32/api/ioapiset/nf-ioapiset-getoverlappedresult).

O wrapper reutilizável `try_send` drena antes de retornar. **Seu orçamento de 3 ms é de tentativa, não garantia de retorno durante cancelamento.** Essa distinção é parte do contrato público e da documentação: memória segura não pode ser trocada por uma falsa promessa de término de I/O cancelado. Não há reaper por chamada, leak ou número ilimitado de threads. Caller que precisa garantir retorno usa processo descartável ou uma API assíncrona com ownership explícito; esta rodada não introduz reaper persistente.

Auditar todos os callsites atuais (`doctor`, `stop`, `emit_quota`, `local`, startup/stop CLI e benchmarks). Os testes/probes executam CLI em subprocesso com timeout externo e cleanup. APIs de biblioteca mantêm contrato explícito de drenagem. Nenhum caller de hook pode usar inadvertidamente o wrapper que drena sem limite. Uma espera longa observada continua sendo falha de latência registrada, não sucesso escondido pela segurança da memória.

Decisão de escopo desta rodada: o hook migra obrigatoriamente à API de token; os demais callsites preservam o wrapper seguro e sua possível latência de cleanup, documentada no rustdoc. O integrador lista os callsites auditados na evidência de T02. Não certificar `try_send` como deadline de retorno para callers persistentes nem adicionar comportamento de process exit dentro da biblioteca reutilizável. Uma futura API persistente com cancelamento desacoplado é trabalho separado.

### Hook descartável

`main` obtém instante/deadline e inicia watchdog antes de resolver argumentos, pois o watchdog não precisa conhecer a resposta. Depois resolve o header e é o único escritor: escreve/flush a resposta estática ANTES de ler stdin ou iniciar transporte. Resposta depende somente dos argumentos, não do resultado da telemetria. Se a escrita falhar, terminar exit 0 sem iniciar transporte. Passar o mesmo deadline absoluto à tentativa IPC.

Resposta exata para PreToolUse de Antigravity/Grok/Pi (e adapters existentes que permitem): `{"decision":"allow"}`; Codex e demais eventos: `{}`. Preservar o mapeamento já existente, inclusive adapter não executado nesta campanha. Watchdog nunca escreve JSON, portanto não existe eleição entre dois escritores. Remover o antigo `done` que desarmava a proteção antes do término real.

Em `CleanupRequired`, mover owner para `finish_with_guard<T>(guard) -> !`; conservar a variável viva e terminar sem unwind, sem escrever novamente. O processo descartável retém frame/OVERLAPPED até o término. Não usar callback que retorna nem `panic!` como mecanismo de saída. A saída normal usa `process::exit(0)`, que não executa os destructors Rust das stacks. [Contrato Rust](https://doc.rust-lang.org/std/process/fn.exit.html).

No deadline, watchdog somente solicita término do próprio processo Windows por `TerminateProcess(GetCurrentProcess(), 0)`, sem stdout, logging ou locks Rust. Ele permanece armado inclusive durante emissão do observer. API fica encapsulada no adapter Windows do IPC; demais plataformas conservam saída nativa com testes separados. A API encerra threads e solicita cancelamento de I/O, mas a conclusão do processo ainda depende do kernel: não afirmar retorno hard-real-time de 3 ms. [TerminateProcess](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-terminateprocess).

A obrigação de JSON completo pressupõe stdout válido e consumido pelo harness. Com consumidor bloqueado ou pipe quebrado, watchdog ainda solicita término; missing/partial JSON é falha de resposta, não PASS. Não criar uma espera infinita no watchdog para preservar bytes. A01 inclui compatibilidade por adapter: bytes disponíveis antes do exit devem ser aceitos sem execução dupla, erro de protocolo ou término prematuro que destrua a entrega. Usar configuração de harness isolada quando suportada, nunca hooks globais. Adapter não exercitado fica não medido e não é certificado por mocks. Se algum adapter rejeitar resposta antecipada, gate de compatibilidade falha e a arquitetura deve ser revista antes de promoção; não inserir fallback silencioso que recrie dois escritores.

O deadline padrão continua 3 ms; override de benchmark deve ser explícito no relatório e não certifica o default. Windows não fornece garantia hard-real-time com thread/sleep: registrar atraso real do watchdog. Para o teste A01, usar 100 processos por caso de corrida, timeout externo de 3 s como contenção do harness, exigir resposta única válida nos casos de stdout normal e exit 0. Timeout externo atingido reprova; esse teto não substitui os SLAs de 1/3 ms. Stdin inválido/truncado pode resultar em rejeição pelo daemon: isso é fail-open do hook, não entrega aprovada.

### Conversões de deadline

Para WaitNamedPipe, restante positivo inferior a 1 ms termina tentativa imediatamente; para restante maior, floor em ms sem passar zero. Deadline esgotado não chama a API. Zero em GetOverlappedResultEx é poll válido, sem reinterpretar semântica entre funções. Conferir deadline depois das chamadas; esperar uma instância disponível não garante que o open seguinte terá sucesso. [WaitNamedPipeW](https://learn.microsoft.com/en-us/windows/win32/api/namedpipeapi/nf-namedpipeapi-waitnamedpipew).

### Aceitação adicional A02

Modelo de estados injetável cobre submitted/pending/cancel_requested/completed e impede liberação precoce por construção. Teste nativo usa pipe aceito sem leitura e payload suficiente para forçar I/O pendente; exige evidência de ERROR_IO_PENDING, não assume que o forçou. Se não conseguir, marca o caso não exercitado. Servidor de teste é sempre liberado/encerrado pelo harness, impedindo travar a suíte no Drop. Repetição de cancelamento verifica handles/operações vivas retornando à linha de base; ausência de crash sozinha não prova segurança. Não usar memória inválida deliberadamente no processo do coordenador.

## 11. Medição interna no candidato exato

Adicionar observer opt-in ao mesmo binário release, ativado por `AGENT_OTEL_BENCH_HANDLE`, handle de pipe anônimo herdado fornecido pelo harness. Validar formato e tipo do handle; variável ausente usa caminho normal. Sem arquivos ou campos de tracing no payload. Especificar o registro binário com magic `AOBT`, versão 1, tamanho fixo, PID, flags e durações u64 em nanos; o harness mapeia PID ao evento esperado. Cada processo pode produzir no máximo um registro.

Layout fechado de 40 bytes, little-endian: magic[4], version u16=1, size u16=40, pid u32, flags u32, response_completed_ns u64, before_transport_ns u64, work_completed_ns u64. Flags bit0=normal_completion, bit1=send_completed; outros bits zero nesta versão. Exigir response_completed <= before_transport <= work_completed, todos deltas de main_entry. Handle recebido é decimal u64 representável em HANDLE, validado como pipe; valor inválido desabilita observer e produz diagnóstico de ausência no harness. O pai usa lista explícita de handles herdáveis, drena continuamente e limita captura a 40 bytes por filho. Registro duplicado, versão inválida ou PID inesperado reprova o observer.

Cronômetro monotônico inicia na primeira operação de main e termina após a conclusão normal da tentativa de transporte; a resposta já foi escrita antes do stdin. Essa fronteira inclui parsing de argumentos, resposta, stdin, envelope e tentativa IPC; exclui loader/criação do processo, emissão do registro e teardown após exit. CleanupRequired não produz registro normal. Emitir o registro depois da região medida para o pipe já drenado pelo harness. Caso o registro bloqueie, watchdog solicita término: sample ausente é `not_measured`, nunca zero nem exclusão silenciosa do denominador.

Medir modo observer habilitado no hash exato do candidato e reportar esse modo. Não alegar que isso observou diretamente o caminho desabilitado. Fazer comparação externa pareada com observer ausente para detectar perturbação, sem subtrair médias para fabricar aprovação. Verificação do tamanho usa o mesmo binário. A assertion interna vale para a fronteira/modo observados e exige todos os samples normais válidos abaixo de 1.000 µs; p99/max também são reportados. Se houver ausência de amostra, erro de saída ou deadline, não aprovar.

O modo watchdog/timeout é diagnóstico separado e não entra escondido como sample rápido: contabilizar todos os processos iniciados e seu resultado. Confirmar funcionamento default com stdout normal, sem observer e sem override. Se o observer não couber no orçamento ou não produzir medição íntegra, T03 devolve o gate reprovado/não medido; não substituir o SLA por duração externa e não remover o gate para encerrar a campanha.

## 12. Revisão desta especificação

Revisões Codex: Sol high para ownership/watchdog, Sol medium para pipeline/contexto e Luna medium para validação. A integração corrigiu limites de JSON expandido, licença de bytes durante HTTP, ausência de cwd ambiente no novo builder, resultado OTLP desconhecido e ledger de campanha. A revisão final do IPC levou à resposta antecipada com um escritor e watchdog sem stdout; sua compatibilidade é um gate de candidato, não fato já validado em todos os harnesses.

O manifesto foi verificado como JSON, DAG acíclico, cobertura de R01–R11/A01–A12, referências locais existentes e ausência de arquivos com dois escritores sem ordem de dependência. Nesta rodada de documentação não foram executados Cargo, novos benchmarks ou alterações da instalação. Implementabilidade do contrato não equivale à aprovação empírica de tamanho, latência ou entrega.
