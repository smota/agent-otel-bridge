# M2 — Codex real após revisão de confiança

Data: 2026-10-06. Base da entrega: 723f7f0. Resultado: aceite da correção dos três hooks no ambiente CLI testado; M2 permanece aberto.

## Prova observada

Samuel informou ter habilitado os hooks. Consulta `hooks/list` confirmou os três hashes da projeção aplicada como trusted, sem erros ou warnings. Nenhuma confiança foi alterada pelo coordenador.

Uma sessão real via app-server, `gpt-5.6-luna` medium e subscrição, executou exatamente uma leitura do marcador. Comando exit0, resposta e conteúdo iguais, turno completed.

| Evento | Resultado do Codex | Duração reportada | Spans no receptor |
| --- | --- | --- | --- |
| PreToolUse | completed | 947ms | 1 |
| PostToolUse | completed | 839ms | 1 |
| Stop | completed | 933ms | 1 |

Três spans com trace `4d17f73e45eb48d234618113fd0ef4b1`, três span IDs distintos e sessão `01a10f85-7b17-7420-bcd5-be0301939051`. Todos atribuídos a codex/openai/gpt-5.6-luna. Pre/Post são `execute_tool Bash`; Stop é `agent.stop`. Os IDs de execução Pre/Post também coincidem nas notificações do harness.

Daemon: três eventos aceitos, transformados e enfileirados para exportação; zero rejected, invalid, unknown e shutdown_dropped. Três requisições de traces decodificadas e recebidas. Os cinco frames ingress incluem os dois frames de controle do supervisor; não são cinco eventos de agente.

## Limites

- A estação agora usa Codex **0.160.1**, SHA `9e7c59c05cc1ce5677b1f94e835b2ac038ca3be14504e78d558eacdb0ea3f55d`. Diagnóstico anterior usava0.154.0. A correção funciona na configuração atual; a comparação antes/depois não isola a atualização do harness como variável.
- Durações incluem execução pelo shell e não medem o SLA nativo <1ms. A amostra real ficou acima da mediana sintética anterior; três eventos não definem distribuição ou p99.
- Correlação comprovada por sessão/trace. `gen_ai.tool.call.id` e parent span continuam ausentes, contexto `missing`. Não certifica correlação geral, parentesco ou contexto de workspace.
- Receptor OTLP privado; não certifica SigNoz, dashboards, persistência do daemon, Desktop/IDE ou subagentes.
- Binários ativos0.5.2 preservados. Hooks e configuração Codex/Claude preservados durante o teste. Daemon/harness encerrados exit0, nenhum pipe residual.

## Receitas e evidências

Reutilizadas sem alteração as receitas [supervisor](codex-execution-01/aob-codex-hook-supervisor.py), [cliente RPC](codex-execution-01/aob-codex-hook-execution.py) e [decoder](real-cli/decode.py). Supervisor com job Windows kill-on-close, timeout150s, pipe exclusivo e chaves de API retiradas do filho; sandbox readOnly/networkAccessfalse. Uma inferência, nenhuma repetição nem escalada de modelo.

[Trust](codex-real-02/hooks-list.json), [harness](codex-real-02/harness.json), [spans selecionados](codex-real-02/spans.json), [diagnósticos](codex-real-02/diagnostics.json), [recibo](codex-real-02/receipt.json), [hashes](codex-real-02/sha256.json). Corpos OTLP e streams brutos não entram no Git. Asserções conferiram contagens, sessão/trace, resultados, hashes, configuração e cleanup.

## Encaminhamento

Validação de entrega: `cargo guardrails` passou em64,1s (fmt, Clippy, workspace tests, conformance, docs, cliente152576bytes e branch). `native_context_process` permanece ignored pelo contrato existente. Hashes das evidências e links locais conferidos; processos de teste ausentes e corpos/streams brutos temporários removidos após extração.

Fechado o defeito de execução dos três hooks nesta superfície. Próxima unidade M2: lifecycle Grok, começando pela origem dos dois Stop e pela guarda dos hooks Claude importados. A discussão solicitada sobre eventos ausentes foi registrada em [cobertura de lifecycle](../../LIFECYCLE-REVIEW.md), com decisão M4 e implementação M6 antes de finalizar dashboards.
