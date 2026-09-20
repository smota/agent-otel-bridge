# Quebra de implementação por camada

Complementa [`README.md`](README.md) (raciocínio e restrições) e [`decisions.md`](decisions.md) (decisões e alternativas descartadas).

Cada camada traz: job to be done, entregável, arquivos, testes, impacto nos gates e riscos. Nenhuma camada está iniciada.

Ordem recomendada: **L1 → L2 → L4 → L3**. L4 antes de L3 porque é zero-code e não depende dele.

Gates aplicáveis a toda entrega (`AGENTS.md` §4): `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, `cargo test --workspace`, `cargo doc --workspace --no-deps`, `cargo guardrails`. Atributos novos exigem entrada em `docs/TELEMETRY_DICTIONARY.md` e preservação de compatibilidade com os dashboards existentes.

---

## L1 — `exec` wrapper

**JTBD.** Instrumentar qualquer ferramenta do entorno sem alterar o código dela, e completar a metade *egress* do contrato de propagação já descrita em `AGENTS.md` §3.4 e nunca implementada.

**Entregável.** `agent-otel-bridge exec [--domain <prefixo>] -- <cmd> [args...]`

Sequência:

1. Resolve `trace_id`/`span_id` (contexto herdado via `TRACEPARENT`, ou raiz nova).
2. Injeta `TRACEPARENT=00-{trace_id}-{span_id}-01` no ambiente do filho.
3. `spawn()` com `Stdio::inherit()` — **antes** de qualquer tentativa de IPC.
4. `wait()`, capturando duração e exit code.
5. Classifica o arquétipo do argv (`archetype.rs`, já existente).
6. Notifica o daemon, best-effort, dentro do orçamento de IPC existente.
7. Sai com o exit code do filho; `128+sinal` se morto por sinal.

**Arquivos.** `crates/agent-otel-cli/src/main.rs` (subcomando), novo `crates/agent-otel-cli/src/exec.rs`.

**Testes.** Propagação de exit code (incluindo não-zero); comportamento sob sinal; passthrough de stdin/stdout/stderr; daemon ausente não impede execução nem altera exit code; `TRACEPARENT` presente no ambiente do filho; `TRACEPARENT` herdado produz parentesco correto.

**Gates.** Impacto no binary size do `agent-hook`: **zero** — código novo vive no CLI, e `guardrails.rs:124-129` mede exclusivamente `agent-hook{EXE_SUFFIX}`. O CLI não tem SLA de tamanho.

**Desbloqueia.** Gate/orquestrador de loop, step de CI (o arquétipo `build_test_verify` já existe), eval harness.

**Riscos.** Semântica de sinais difere entre Unix e Windows — o `platform_conformance` precisa cobrir os dois. Não usar `execve`-replace: impediria capturar duração e exit code.

**Por que é o primeiro.** Reusa código já testado (archetype, context harvester, derivação de IDs), não toca no binário sob o invariante mais caro, implementa um contrato já escrito, e não depende de nenhum ponto em aberto.

---

## L2 — Contrato de ingestão

**JTBD.** Transformar o protocolo IPC de detalhe interno em contrato versionado, para que cada linguagem nova seja um arquivo pequeno em vez de um port do produto.

**Entregável.**

- `MsgType` novo para domain span (slots `0x05`–`0xFE` livres, `frame.rs:21-27`).
- Payload: nome do span (limitado em tamanho e charset), status (`OK`/`ERROR`/`UNSET` + mensagem opcional), atributos tipados (string/int/double/bool), IDs externos opcionais, reuso do envelope de contexto/`traceparent` existente.
- Validação de prefixo reservado na borda (README §6.2).
- Wire protocol documentado como contrato público, com versionamento (ver A3).

**Arquivos.** `crates/agent-otel-ipc/src/frame.rs`, `crates/agent-otel-core/src/model.rs` (struct de domain span, **sem tocar** `HookEvent`), `crates/agent-otel-core/src/semconv.rs` (lista de prefixos reservados), `crates/agent-otel-core/src/otlp.rs` (`build_span_from_domain`, paralelo a `build_span_from_resolved`), `crates/agent-otel-daemon/src/transform.rs`.

**Testes.** Roundtrip de encode/decode do frame; rejeição de cada prefixo reservado; rejeição de nome de span fora do limite; status mapeado corretamente para o `Status` do OTLP; aceitação de IDs externos; frame malformado não derruba o daemon.

**Gates.** Zero impacto no `agent-hook` — o cliente não compõe esse frame. Atenção a `AGENTS.md` §3.1: `agent-otel-core` não pode ganhar dependências pesadas.

**Riscos.** Payload de tamanho variável exige revisar limites de frame e o orçamento do micro-batch (50 spans / 200 ms). Definir teto explícito e documentá-lo.

---

## L4 — `mcp-proxy`

**JTBD.** Fazer o contexto do turno do harness chegar ao MCP server, para que spans já emitidos por servidores instrumentados se costurem sob o turno correto.

**Entregável.** `agent-otel-bridge mcp-proxy -- <server cmd>`: shim stdio que lê `TRACEPARENT` do ambiente, injeta em `params._meta` de requests e notifications conforme SEP-414, e emite o span client-side com atributos `mcp.*` (D1).

**Arquivos.** Novo `crates/agent-otel-cli/src/mcp_proxy.rs`; constantes `mcp.*` em `crates/agent-otel-core/src/semconv.rs`; entrada em `docs/TELEMETRY_DICTIONARY.md` marcando a convenção como Development.

**Testes.** Passthrough byte-exato quando a instrumentação falha; JSON-RPC malformado não corrompe o stream; `_meta` preexistente sofre merge e `traceparent` preexistente **não** é sobrescrito; injeção só em requests e notifications, nunca em responses; batches JSON-RPC preservados; exit code e sinais propagados; server que fecha stdout não trava o proxy.

**Gates.** Zero impacto no `agent-hook`. `mcp.*` entra no dicionário com marcação explícita de instabilidade.

**Desbloqueia.** O fan-out hoje invisível dentro de uma chamada de MCP — a pergunta "por que este MCP levou 2,3 s" passa a ter resposta quando o server é instrumentado por qualquer meio.

**Riscos.** O overhead do hop de processo é desconhecido e precisa de medição antes de recomendação ampla (A1). O proxy é componente crítico no caminho: as restrições de README §6.5 são obrigatórias. SEP-2028 (`_meta` → headers HTTP) pode alterar expectativas para servers HTTP; acompanhar.

---

## L3 — Clientes de instrumentação em código

**JTBD.** Permitir que um MCP server, um gate ou uma ferramenta emita semântica própria e ganhe, de graça, o contexto que o bridge tem e o processo não.

**Entregável.** Clientes finos em Python e TypeScript (D3), sem dependências, escrevendo no mesmo socket/named pipe, fail-open por construção. Crate `agent-otel-api` para uso in-process no workspace Rust.

Superfície mínima por cliente: iniciar span com nome e prefixo de domínio; anexar atributos tipados; definir status; encerrar; ler `traceparent` do ambiente ou de `params._meta`.

**Arquivos.** Diretório novo fora de `crates/` para os clientes não-Rust; `crates/agent-otel-api/` para o Rust.

**Testes.** Paridade de comportamento entre Python e TypeScript contra o mesmo conjunto de casos do contrato (A5 — a paridade precisa ser verificada por teste, não por disciplina); fail-open com daemon ausente; rejeição de prefixo reservado no cliente, espelhando a validação do servidor.

**Gates.** Os clientes não-Rust ficam fora de `cargo test --workspace`; precisam de gate próprio no CI. Isso é trabalho novo de infraestrutura, não apenas código.

**Riscos.** É o maior compromisso de manutenção do conjunto — dois clientes com releases e paridade a cada mudança de protocolo. Deliberadamente último.

---

## Fora da sequência: histograma

`loop_decision_confidence`, latência e custo pedem histogramas. Hoje o produto emite apenas `Sum` e `Gauge` via `NumberDataPoint`; `opentelemetry_proto` já expõe `Histogram`, mas encoding, agregação e buckets não existem.

Agregação só pode viver no **daemon** — o único componente com estado. O cliente é stateless e está sob o SLA de 1 ms.

Dimensões restritas a enums fechados (README §6.4). Temporalidade delta por ciclo de export, para não acumular estado sem limite.

Deixado por último: é o maior mecanismo novo e o valor da tese se prova com spans.

---

## Resumo

| Camada | Entregável | Impacto no `agent-hook` | Depende de | Bloqueios em aberto |
|---|---|---|---|---|
| L1 | `exec` wrapper | zero | — | nenhum |
| L2 | contrato de ingestão | zero | L1 | A3 (versionamento) |
| L4 | `mcp-proxy` | zero | L2 | A1 (overhead), A2 (saída) |
| L3 | clientes Python/TS/Rust | zero | L2 | A5 (paridade), gate de CI novo |
| — | histograma | zero | — | nenhum |

Nenhuma camada toca o binário sob o SLA de 300 KB nem o caminho de 1 ms. Essa é a propriedade que torna a evolução inteira defensável.
