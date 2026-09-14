# Cobertura da implementação candidata

Este documento mapeia código de teste aos contratos; não armazena resultados de execução. Aprovação exige evidência do candidato identificado. Relatórios e ledgers são temporários. Guardrails verdes não substituem os SLAs nem os casos ainda não cobertos.

| Caso | Implementação verificável | Limite da cobertura |
|---|---|---|
| A01 | `crates/agent-otel-client/tests/hook_contract.rs`, `crates/agent-otel-ipc/tests/ipc_tests.rs` | Resposta antecipada, watchdog, ausência de servidor e contenção; compatibilidade com cada harness real ainda exige execução nativa. |
| A02 | `pending_write.rs` testes de estado, `client.rs` testes de deadline, `ipc_tests.rs` handles e waiters concorrentes | Não existe injeção determinística de todas as combinações de conclusão parcial e `CancelIoEx/NOT_FOUND`. O teste de 250 ms é funcional, não prova do SLA de 3 ms. |
| A03 | `server.rs` testes de licenças/readers; `pipeline.rs` saturação da exportação; `model.rs` limites de parsing | A matriz completa de saturação simultânea de conexão, payload, fila e RSS ainda não é um probe da campanha. |
| A04 | `crates/agent-otel-daemon/tests/pipeline_limits.rs` | Exportador bloqueado com ingresso concorrente; bloqueio simultâneo injetado de filesystem e quotas ainda não coberto. |
| A05 | `crates/agent-otel-daemon/tests/context_cache.rs`, `crates/agent-otel-core/tests/pure_pipeline.rs` | Workspaces, atualização e pureza cobertos; não equivale à medição do SLA do harvester. |
| A06 | `crates/agent-otel-daemon/tests/otlp_outcomes.rs`, testes em `exporter.rs` | HTTP real e políticas unitárias; a CLI da campanha ainda reporta o probe agregado de contratos OTLP como não medido. |
| A07 | `pipeline_limits.rs`, testes em `pipeline.rs` | Shutdown limitado com request em voo e reconciliação; não prova todos os padrões de falha do sistema operacional. |
| A08 | `crates/agent-otel-cli/tests/native_context_process.rs` | Processo candidato → IPC → daemon → coletor local. Executar explicitamente com `AGENT_OTEL_TEST_HOOK`; não é execução de um agente comercial. |
| A09 | `examples/performance.rs`, `examples/performance_ipc.rs`, `hook_timing_campaign.py` | Decomposição e timing do hook disponíveis; cache hit/miss e RSS não têm campanha de microbenchmark completa. Métricas puras são diagnósticas, não substituem assertions históricas. |
| A10 | `tests/test_perf_driver.py`, `test_perf_campaign.py`, `test_perf_loop.py`, testes Rust de relatórios e entrega | IDs, contagens, schema, reservas, hashes e fontes antes/depois; dados antigos permanecem históricos com suas limitações. |
| A11 | Captura bounded/Job Object em `perf_campaign.py`; testes Python; snapshots ativos independentes | Controle de processos e comparação da instalação; não autoriza ativação ou alteração de hooks globais. |
| A12 | Laboratório fleet existente, relações e validação de relatórios | Trace longo nativo e consulta SigNoz por MCP/API não são executados pela campanha determinística de desempenho. Exigem a campanha nativa separada. |

O socket Unix abandonado após crash não é removido automaticamente pelo novo servidor: um pathname existente é preservado para não apagar o endpoint de outro daemon. Recuperação segura com exclusão mútua e validação nativa permanece pendente. Linux e macOS não recebem certificação a partir de resultados Windows.
