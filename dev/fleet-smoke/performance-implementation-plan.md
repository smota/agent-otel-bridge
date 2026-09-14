# Plano de implementação e validação

Versão 1, 2026-09-14. Documento normativo desta rodada: [especificação](performance-implementation-spec.md). Distribuição: [work items](performance-work-items.json). Estado: planejado, não executado. Autorização desta rodada: especificar e atualizar o plano. A instalação ativa está fora do escopo.

## Responsabilidade e escolha de modelos

Codex coordena a especificação e os pacotes de implementação; Antigravity continua coordenando a futura execução nativa da frota, com Codex revisando evidência e podendo operar como relay. Não confundir um subagente Codex com Antigravity. O orçamento máximo de cinco tentativas de uma campanha não é número de mensagens de coordenação nem repetições de microbenchmark.

| Papel | Modelo fechado | Reasoning | Uso |
|---|---|---|---|
| Integração/revisão final | modelo da tarefa raiz | herdado | contratos, conflitos, unsafe, veredito |
| IPC e hook | `gpt-5.6-sol` | high | lifetime, Win32, watchdog e integração |
| Core/contexto/pipeline/exporter | `gpt-5.6-sol` | medium | ownership entre etapas e semântica |
| Schema/controller/fixtures/docs | `gpt-5.6-luna` | medium | tarefas limitadas com contratos definidos |
| Comandos, carga, assertions | nenhum | nenhum | Rust/Python determinísticos |
| Coordenação da frota nativa | `gemini-3.8-flash-medium` no Antigravity | medium | continuar nível já necessário na campanha anterior |
| Participantes da frota | IDs fixados no README e preflight | configurado | não mudar modelos durante uma tentativa |

Esses modelos Codex estão disponíveis nesta sessão. A escolha é uma alocação por complexidade, não alegação de preço público comparado. A orientação oficial admite delegação em subagentes e calibração da verificação à tarefa; disponibilidade efetiva e uso são registrados pelo ambiente. [OpenAI model guidance](https://developers.openai.com/api/docs/guides/latest-model).

Luna recebe no máximo duas rodadas de correção para uma mesma divergência; escalar para Sol medium se não fechar. Sol high recebe revisão da raiz antes de aceitar unsafe. Não escalar para modelos ou fornecedores não disponíveis, não substituir silenciosamente. Continuar trabalho independente se um modelo não estiver disponível e marcar a task dependente como bloqueada. Claude permanece excluído.

## Gates e tarefas

| Task | Dependências | Arquivos principais | Saída e aceitação |
|---|---|---|---|
| T01 — contratos e baseline | nenhuma | `dev/fleet-smoke/performance-*`, fixtures e testes | congelar R01–R11, A01–A12, corpus/hashes; reproduções não destrutivas |
| T02 — I/O seguro | T01 | `ipc/src/client.rs`, novo `ipc/src/pending_write.rs`, `ipc/tests/ipc_tests.rs` | R02, A02; sem buffers liberados enquanto pendentes |
| T03 — hook e medição interna | T02 | `client/src/main.rs`, testes de hook, probe dev | R01/R11, A01/A09; JSON único, exit 0, timing com origem |
| T04 — core puro | T01 | `core/src/model.rs`, `core/src/otlp.rs`, testes | R04/R05/R07; builder sem I/O e compatibilidade de wrappers |
| T05 — snapshots | T04 | novo `daemon/src/context_cache.rs`, `core/src/context.rs`, testes | R05, A05; isolamento, TTL, limites e ausência explícita |
| T06 — exporter | T01 | `daemon/src/exporter.rs`, novo teste `otlp_outcomes.rs` | R06, A06; parcial, retry e deadline |
| T07 — pipeline bounded | T02,T05,T06 | `ipc/src/server.rs`, `daemon/src/daemon.rs`, `batch.rs`, `config.rs`, módulos novos | R03/R04, A03/A04/A07; integração única e shutdown |
| T08 — contabilidade | T07 | novo `daemon/src/diagnostics.rs`, dicionário e testes | R09; razões finitas, conservation por estágio |
| T09 — schema/controller | T01 | scripts perf, novo schema, testes Python/Rust | R09/R10, A10/A11; cinco tentativas e proveniência |
| T10 — probes completos | T03,T07,T08,T09 | exemplos/probes e testes dev | A01–A12; negativas independentes, não validadores tautológicos |
| T11 — integração e qualidade | T10 | revisão workspace, Cargo.lock somente se necessário | fmt/clippy/tests/docs/guardrails e ausência de mutação ativa |
| T12 — campanha candidata | T11 | recursos transientes, sem artefatos de runtime em Git | até cinco tentativas, relatório final com cobertura e limitações |

Um único agente escreve cada arquivo por vez. T02, T04, T06 e T09 podem avançar em paralelo após T01, respeitando três workers mais a raiz. T05 começa após T04. T07 é integração serial: ninguém modifica simultaneamente `daemon.rs`, `config.rs`, `lib.rs` ou manifests. T08 entra depois de T07. A raiz é dona de Cargo e do lockfile durante toda a rodada; agentes pedem verificação à raiz, não iniciam builds concorrentes no checkout.

Não considerar diffs anteriores da suíte como trabalho descartável. Antes de cada task, ler status e registrar arquivos atribuídos. Nunca stash/reset/checkout do trabalho de outro agente. A raiz integra por arquivos autorizados; não há necessidade de commit para validar candidatos.

### Contrato de handoff para todo subagente

Entrada obrigatória: ID da task, versão da especificação, requisitos/testes, lista de arquivos permitidos, dependências concluídas, invariantes e modelo. Saída obrigatória: arquivos alterados, contratos implementados, testes executados e resultados com contagem, testes não executados, limitações, quebra de compatibilidade se houver e pedido de revisão específico. Não afirmar PASS com compilação apenas, teste ignorado, texto do LLM ou mock que contorna a fronteira testada.

Se uma correção exigir mudar contrato ou SLA, devolver proposta à raiz antes de adaptar expectativas. A raiz pode resolver detalhes locais compatíveis; uma mudança em R01–R11 ou nos limites declarados exige atualizar versão/justificativa da especificação e reexecutar os testes afetados. Não mover silenciosamente uma obrigação para “fora do escopo”.

## Comandos de verificação

Comandos existentes, a partir da raiz, executados sequencialmente pelo integrador:

```text
python -m unittest discover -s dev/fleet-smoke/tests -p "test_perf_*.py"
cargo test -p agent-otel-core
cargo test -p agent-otel-ipc
cargo test -p agent-otel-daemon
cargo test -p agent-otel-fleet-smoke
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo doc --workspace --no-deps
cargo guardrails
```

Rodar testes de pacote conforme a task, depois checks finais do workspace; não repetir toda a lista sem novo motivo. Guardrails pode repetir verificações internamente; respeitar esse gate final do projeto. Novos testes nomeados por Axx ainda precisam ser criados: a existência futura de um comando filtrado não significa que o teste existe hoje.

Builds candidatos existentes:

```text
cargo build --release -p agent-otel-client -p agent-otel-bridge
cargo build -p agent-otel-bridge
cargo build --release -p agent-otel-fleet-smoke --example performance
cargo build --release -p agent-otel-fleet-smoke --example performance_ipc
python dev/fleet-smoke/perf_campaign.py --repeats 3
```

O último comando mede componentes disponíveis hoje; ainda não implementa todas as assertions desta especificação. `--compose-only` apenas reavalia relatórios, sem gerar nova medição. Executáveis são resolvidos por Python com sufixo da plataforma; não criar comandos PowerShell como API da suíte. O novo `--suite` só pode aparecer como comando executado depois de T09/T10.

## Loop fechado da campanha candidata

Controlador novo `perf_loop.py` mantém estado em memória, emite JSONL para stdout e pode usar ledger em diretório temporário próprio durante a sessão. Não agenda heartbeat, cron ou execução após encerrar a tarefa. Um processo reiniciado não retoma silenciosamente nem ganha mais cinco tentativas: continuação exige `--resume-ledger` explícito com o mesmo campaign ID, histórico e contador; sem ledger válido, reportar impossibilidade de retomar. Criar outra campanha exige decisão explícita do coordenador e registro da relação anterior, nunca fallback automático.

CLI nova: `python dev/fleet-smoke/perf_loop.py --campaign-kind performance --seed 42 --campaign-dir <owned-temp-dir>`. A raiz cria esse diretório com tempfile e conserva sua propriedade até avaliação final. O script não executa LLM nem edita produto: em `repair`, grava ledger atomicamente dentro desse diretório, limpa subprocessos, emite `needs_repair` e retorna código 4. A raiz aplica a correção e verificações previstas; depois chama `--resume-ledger <ledger.json>` com nova proveniência. Resume valida schema, histórico, hashes das evidências referenciadas e limites acumulados; nunca executa comandos arbitrários armazenados no JSON. Snapshot de candidato alterado é registrado como transição, não sobrescrito no histórico.

Estados terminais passed/failed/not_measured usam os códigos da especificação; ledger incompleto por crash pode ser retomado somente classificando tentativa reservada como interrompida e consumida. Ausência de contexto de autorização no coordenador impede nova execução, não permite presumir continuação. Diretório de campanha é transitório, sem recuperação durável garantida: perda do ledger encerra a campanha como incompleta. Cleanup final remove somente os arquivos próprios depois de reportar evidência; não gravar resultados no Git.

Estados: `prepare → verify → reserve → run → observe → assess → cleanup → decide`. Saídas de decide: `complete`, `repair`, `blocked` ou `exhausted`. `repair → verify` exige nova revisão/hashes; não alterar candidato durante run/observe. Registrar transições, timestamps e motivo. Exceção/cancelamento passa por cleanup/finalize em finally e gera relatório parcial.

Prepare/verify não consomem tentativa enquanto nenhum cenário iniciar. Reservar índice monotônico 1..5 imediatamente antes do primeiro subprocesso de cenário. Falha depois desse ponto consome tentativa, inclusive timeout. Repetições estatísticas pré-declaradas pertencem à tentativa; não são retries gratuitos de um cenário que falhou. Nenhuma camada interna repete uma invocação paga automaticamente.

| Tentativa | Conteúdo default se anterior aprovada | Após falha corrigível |
|---|---|---|
| 1 | regressão funcional, IPC/contexto/OTLP e baseline decomposta, seed 42 | não se aplica |
| 2 | falhas injetadas: cancelamento, limites, exporter lento, shutdown, seed 42 | repetir menor cenário que falsifica a hipótese, mesma seed |
| 3 | performance completa e entrega debug/release, seed 42 | repetir o afetado após patch e checks |
| 4 | confirmação independente do candidato congelado, seed 43, cobertura restante | confirmar correção sem trocar seed e código ao mesmo tempo |
| 5 | reserva para evidência obrigatória/última confirmação | encerrar com pendências se não houver evidência suficiente |

Cada tentativa tem teto de 15 minutos incluindo observação/cleanup; no máximo 75 minutos acumulados de cenários. Preparação/correções têm budget operacional de 60 minutos para essa campanha, distinto do trabalho de implementar T01–T11. Deadline de cleanup reserva 10 s no orçamento e encerra somente processos filhos próprios; confirmar que não são processos da instalação ativa. Ao consumir teto, reportar incompleto; não mascarar tarefas restantes como concluídas.

Falha reproduzível e compatível com escopo permite patch mínimo de candidato. Falha externa inalterada (auth, quota, backend ausente) bloqueia caminho dependente sem gastar todas as tentativas. Resultado melhora após alteração: preservar histórico reprovado e certificar somente o novo candidato se todas as assertions obrigatórias nele passarem; nunca combinar PASS de hashes diferentes para inventar cobertura completa. Última modificação invalida apenas evidência afetada, com mapa explícito de dependências, e exige confirmação final do conjunto integrado.

Não confundir esta campanha de desempenho com a sequência live baseline/mixed/long-trace do [plano da frota](first-live-run-plan.md). O controlador possui `campaign_kind=performance|fleet`, cada qual com seu ledger, sem abrir outra campanha automaticamente ao esgotar cinco. A12 pode ser exercitada deterministicamente; prova de ferramentas reais dos três agentes exige campanha fleet coordenada pelo Antigravity e orçamento explícito dessa campanha.

Observação SigNoz: API/MCP, nunca UI. Buscar pelo trace ID e janela explícita; consultas no máximo aos 0, 2, 5, 10, 20, 40 e 60 s após flush, timeout 5 s por consulta e teto de 65 s. Encerrar cedo ao completar cobertura. Falha da consulta é `query_failed`, não ausência comprovada de spans. Resultados locais continuam válidos com backend indisponível, mas visibilidade obrigatória fica `not_measured`.

## Definição de concluído

Especificação concluída quando tarefas, contratos, defaults, DAG e acceptance IDs forem consistentes e revisados. Implementação concluída quando T01–T11 passarem e diffs forem revisáveis. Validação concluída quando T12 produzir avaliação por requisito para um candidato identificado; pode concluir a execução com resultado failed/not_measured. Produto aprovado em desempenho exige todas as assertions obrigatórias passed. Instalação/promoção continua sendo uma ação separada e não pertence a este plano.
