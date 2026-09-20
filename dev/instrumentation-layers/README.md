# Camadas de instrumentação — registro de desenho e backlog

Estado: desenho aceito, nenhuma implementação iniciada. Este documento é o registro de raciocínio de uma discussão de arquitetura sobre até onde o `agent-otel-bridge` deve ir além da instrumentação automática que faz hoje. Ele existe para que o desenvolvimento incremental não precise reconstruir o contexto a cada retomada.

Escopo do documento: por que evoluir, o que exatamente se ganha, o que fica explicitamente fora, quais restrições de desenho vinculam cada camada, e o que ainda está em aberto. Não é compromisso público de roadmap — é documento de trabalho em `dev/`, pela mesma convenção de `dev/fleet-smoke/*-plan.md`.

Índice:

- [`decisions.md`](decisions.md) — decisões tomadas, com alternativas descartadas e justificativa.
- [`breakdown.md`](breakdown.md) — quebra por camada: entregável, arquivos, testes, gates, riscos.
- [`execution-plan.md`](execution-plan.md) — quem executa cada item: papel, tier de modelo e modo (cross-harness ou cross-model), aplicando o [modelo de execução](../execution-model/README.md).

---

## 1. Job to be done

**Para quem instrumenta um loop de engenharia, um MCP server ou uma ferramenta no entorno do agente: obter um trace único, que atravesse o turno do harness até a chamada de rede mais interna, sem adotar um SDK OTel completo em cada processo e sem perder o contexto de repositório, workspace e turno que só o bridge conhece.**

Duas perguntas que hoje não têm resposta e que definem o sucesso:

1. *"Por que esta chamada de MCP levou 2,3 segundos?"* — hoje o bridge vê a fronteira da chamada e nada dentro dela. Um `mcp__github__search_code` pode conter doze chamadas de API; todas invisíveis.
2. *"Este passo do meu gate foi confiante e errado — mostre a decisão, o override, o oráculo e o ator."* — hoje não há como um processo externo ao harness emitir semântica própria através do bridge.

Critério de pronto do conjunto: abrir um span de turno do harness e descer, no mesmo trace, até uma chamada HTTP feita dentro de um MCP server — sem juntar traces manualmente e sem que o MCP server precise saber o que é o bridge.

---

## 2. Como chegamos aqui

O gatilho foi uma especificação de instrumentação para um **gate de engenharia** externo (loop decide → override → exec → verify, com confiança calibrada, outcome e ECE semanal em Postgres). A pergunta original era se aquilo cabia neste produto.

A análise passou por três posições, e as duas primeiras estavam erradas. Elas ficam registradas porque o erro é informativo e não deve ser repetido:

**Posição 1 — "implementar `loop.*` aqui".** Rejeitada. A própria especificação declarava `service.name=engineering-loop`; um `service.name` distinto significa outro serviço. Vocabulário de domínio de um consumidor não pertence ao `semconv.rs` do core. Absorvê-lo transformaria o bridge em backend de um gate específico.

**Posição 2 — "ponto de extensão passivo".** Insuficiente. Um `MsgType` novo esperando que alguém o use não entrega valor sozinho e não justifica o custo de estabilizar um contrato. Além disso, o desenho proposto nessa posição incluía um ack síncrono no IPC para devolver `trace_id`/`span_id` ao chamador — desnecessário, ver §6.3.

**Posição 3 — a aceita.** O produto tem duas camadas com naturezas distintas, e só uma delas generaliza. A partir daí, a evolução natural é a mesma progressão do OpenTelemetry: **auto-instrumentação → instrumentação em código**, em que a segunda é opcional e enriquece a primeira.

### 2.1 A tese e seus limites honestos

> A categoria do `agent-otel-bridge` não é "hooks de harness". É instrumentação de processos efêmeros no entorno do agente — qualquer unidade de trabalho que viva de milissegundos a segundos, para a qual um SDK OTel convencional é inadequado (custo de init, flush no exit, estado persistente, configuração em disco).

A tese **se sustenta na camada de enriquecimento**: `archetype.rs` opera sobre uma string de comando e o context harvester sobre um `cwd`. Nenhum dos dois sabe o que é um agente. Mais: um wrapper de execução recebe o **argv cru**, enquanto o hook recebe um nome de evento já processado — o entorno é um consumidor *melhor* do diferencial do produto do que o próprio harness.

A tese **não se sustenta na camada de modelo de evento**, e não deve ser forçada:

| Componente | Generaliza? | Por quê |
|---|---|---|
| `archetype.rs` (classificador de argv) | Sim | Opera sobre string de comando; independe de harness |
| Context harvester (`vcs.*`, `workspace.*`) | Sim | Opera sobre `cwd`; independe de harness |
| `capability.rs` (`mcp__server__tool`) | Parcial | Pressupõe que a chamada já esteja estruturada como tool call |
| `quota.*` | **Não** | É budget de provider de LLM. Não existe quota de um step de build |
| `HookEvent` + nomes de span | **Não** | `otlp.rs:209-231` deriva nomes de `GEN_AI_OPERATION_*` (`semconv.rs:7-10`) — vocabulário GenAI |
| `derive_trace_id(conversation_id)` | Parcial | `conversation_id` é conceito GenAI; para uso genérico o parâmetro é um `correlation_id` (dívida de nomenclatura, não de wire format) |

**Leitura correta:** o bridge é um instrumentador de processos efêmeros **com** uma especialização GenAI em cima. A especialização é valor, não acidente. Não generalizar `quota.*` nem `HookEvent`.

### 2.2 Um erro de unidade de análise, corrigido

MCP server e IDE extension foram inicialmente excluídos por serem "processos de vida longa", o que quebraria a premissa de efemeridade. **A unidade de análise estava errada.** O processo é longo; a **requisição JSON-RPC é efêmera**. Instrumenta-se o boundary de dispatch, não o processo. MCP volta para dentro do escopo. IDE extension permanece fora (§7).

### 2.3 O contrato já está meio escrito

`AGENTS.md` §3.4 especifica a metade *egress* da propagação:

> "When spawning a subagent or running child commands, inject `$env:TRACEPARENT` with format `00-{trace_id:32hex}-{current_span_id:16hex}-01`."

Verificação no código: `TRACEPARENT` é apenas **lido** (`trace_id.rs:172`, `ipc/client.rs:135`, `model.rs:592`). Nenhum ponto do workspace **injeta** a variável em um processo filho. A camada L1 não é direção nova — é a implementação de um contrato já documentado e nunca cumprido.

---

## 3. Estado atual: o envelope e o conteúdo

O limite entre as duas camadas é o que o bridge estruturalmente vê e não vê.

| | Auto-instrumentação (hoje) | Instrumentação em código |
|---|---|---|
| **Vê** | Fronteira da chamada, argv, arquétipo, exit code, duração, tokens do turno, quota, `vcs.*`/`workspace.*`, linhagem de agente, `mcp__server__tool` | Sub-chamadas HTTP/DB, cache hit/miss, tipo da exceção na origem, fases de raciocínio, retry/backoff, semântica de domínio |
| **Não vê** | Fan-out dentro de uma chamada, causa da latência, tipo da exceção, semântica de domínio | — |

Auto-instrumentação entrega **cobertura**. Instrumentação em código entrega **decomposição causal**. Nenhuma das duas substitui a outra.

### 3.1 A alavanca real

O ganho não é "o bridge ganha um SDK". É que **o daemon já é um collector-lite com enriquecimento, e o socket já existe**.

Um MCP server em Python que adota o SDK OTel convencional precisa de dependência pesada, exporter, batching no processo host e configuração de endpoint — e ainda produz um span **órfão**, porque não sabe a qual turno do harness pertence nem em que branch/worktree está.

O mesmo servidor escrevendo no socket do bridge ganha, sem configuração: `vcs.*`, `workspace.*`, `host.*`, arquétipo, micro-batching, fail-open — e o span **costurado sob o turno correto**.

> **O código dá profundidade ao bridge; o bridge dá ao código o contexto que o código não consegue obter sozinho.**

Essa reciprocidade é o argumento de produto. É o que significa, concretamente, estar "um nível acima da instrumentação OTel básica".

---

## 4. Contexto externo verificado

Verificado em 2026-09-20 contra fontes primárias. Releva para o desenho porque define o que **não** devemos inventar.

### 4.1 SEP-414 — status **Final**

Standards Track, criado em 2025-04-25, autor Adrian Cole, sponsor Marcelo Trylesinski, PR #414.

- `traceparent`, `tracestate` e `baggage` viajam em `params._meta` de requests e notifications MCP.
- **Sem prefixo DNS** — exceção explicitamente documentada à convenção de prefixação de `_meta`, para não quebrar correlação de traces.
- Valores em formato W3C Trace Context / W3C Baggage.
- Implementações de referência existentes: C# SDK, Python SDK (PR #1693), OpenInference (Python e TypeScript).
- SEP-2028 constrói em cima, encaminhando valores de `_meta` para headers HTTP.

Exemplo não-normativo da própria SEP:

```json
{
  "jsonrpc": "2.0", "id": 2, "method": "tools/call",
  "params": {
    "name": "get_weather",
    "arguments": { "location": "New York" },
    "_meta": { "traceparent": "00-0af7651916cd43dd8448eb211c80319c-00f067aa0ba902b7-01" }
  }
}
```

### 4.2 Semconv OTel para MCP — status **Development**

`mcp.method.name` (required), `mcp.resource.uri` (conditionally required), `mcp.protocol.version`, `mcp.session.id` (recommended), mais `gen_ai.tool.name` / `gen_ai.operation.name`. Nome de span no formato `{mcp.method.name} {target}`.

**Development significa instável.** A decisão de adotá-la mesmo assim está registrada em [`decisions.md`](decisions.md) (D1), com a contrapartida obrigatória de marcação explícita no dicionário de telemetria.

### 4.3 Consequência para o desenho

Não cabe ao bridge inventar propagação para MCP nem instrumentar o interior de MCP servers. Servidores já instrumentados emitem `mcp.*` e extraem o parent de `_meta`.

**O gap real é outro: quem injeta o `traceparent` do turno do harness dentro do `_meta`?** Isso é responsabilidade do *cliente* MCP, que vive dentro do harness — fora do nosso alcance. Existe issue aberta no Claude Code pedindo exatamente isso (anthropics/claude-code#76391).

Daí a forma do L4 ser um **proxy stdio**, não um wrapper de server. Ver D2 em [`decisions.md`](decisions.md).

---

## 5. Arquitetura em camadas

Progressão deliberadamente análoga à do OpenTelemetry: cobertura automática primeiro, profundidade opcional depois.

```
L0  harness hooks → agent-hook → daemon → OTLP           [existe]
L1  exec wrapper: zero-code no entorno, injeta TRACEPARENT
L2  contrato de ingestão: MsgType de domain span, wire protocol público
L3  clientes finos: Python + TypeScript + crate Rust
L4  mcp-proxy: injeta traceparent em params._meta (SEP-414)
```

| Camada | Natureza | Zero-code? | Depende de |
|---|---|---|---|
| L0 | auto | sim | — |
| L1 | auto | sim | — |
| L2 | contrato | n/a | L1 (valida o formato do span de domínio) |
| L3 | código | não | L2 |
| L4 | auto | sim | L2; independe de L3 |

L2 é a peça de arquitetura: transforma o protocolo IPC de detalhe interno em **contrato versionado**. Depois dela, cada linguagem é um arquivo pequeno em vez de um port do produto.

L4 **não depende de L3**. O proxy injeta contexto e emite o span client-side sozinho; instrumentação dentro do server é ganho adicional, não pré-requisito.

---

## 6. Restrições de desenho

As mais importantes do documento. Cada invariante do `AGENTS.md` tem um escopo preciso, e confundir escopo foi a origem de metade dos erros desta discussão.

### 6.1 Escopo real de cada invariante

| Invariante | Vincula o quê | **Não** vincula |
|---|---|---|
| `< 1.0 ms` de execução | `agent-hook` | CLI, daemon, proxy, clientes de linguagem |
| `< 300 KB` de binário | `agent-hook` — verificado: `guardrails.rs:124-129` mede especificamente `agent-hook{EXE_SUFFIX}` | Qualquer outro binário. O CLI já hospeda `daemon`, `doctor`, `benchmark`, `scan-all` sem SLA de tamanho |
| Zero subprocesso | a **via de telemetria** (não fazer `git.exe` para enriquecer um span) | Um wrapper cujo *propósito* é executar um subprocesso |
| Zero disk I/O no hot-path | `agent-hook` | daemon (já mantém cache em memória e lê `.git/HEAD`) |
| Fail-open | tudo | — (é universal) |
| Sem deps pesadas (`tokio`, `reqwest`, `clap`) | `agent-otel-core`, `agent-otel-client` | CLI e daemon, que já as usam |
| Semconv estrita + retrocompat de atributos | constantes em `semconv.rs` | namespaces de domínio de terceiros (ver 6.2) |

O ponto que destrava o L1: **"zero subprocesso" é sobre a via de telemetria**. Um wrapper de execução não é telemetria emitindo subprocesso; é um subprocesso que emite telemetria como efeito colateral. Não há conflito.

### 6.2 Governança de namespace

Regra: o bridge **nunca** hardcoda vocabulário de domínio de terceiros. Nem `loop.*`, nem `ci.*`, nem `deploy.*`.

O mecanismo é de **prefixo reservado**, validado na borda: rejeitar qualquer chave que comece por `gen_ai.`, `agent.`, `capability.`, `vcs.`, `workspace.`, `service.`, `host.`, `os.`, `deployment.`, `user.`, `terminal.`, `mcp.`. Normalizar para lowercase ASCII antes de comparar. Qualquer outro prefixo é aceito sem que o produto o conheça.

**Furo conhecido e aceito:** a validação protege a *chave*, não o *valor*. Nada impede alguém de escrever `ci.operation.name = "invoke_agent"` e confundir dashboards por convenção. Isso não se resolve em código; resolve-se mantendo a lista de prefixos fechada e por revisão de PR. Registrado para não ser redescoberto como bug.

Corolário: `HookEvent` **não ganha um sexto valor**. O domínio genérico trafega por um `MsgType` novo, não pela enum fechada (`model.rs:19-26`), preservando o parser rápido do caminho de hook.

### 6.3 Identificadores: inverter o fluxo, não adicionar ack

O IPC é fire-and-forget: `agent-hook` escreve e sai sem ler resposta. Isso foi inicialmente lido como bloqueio para devolver `trace_id`/`span_id` ao chamador (necessário, por exemplo, para gravá-los numa tabela externa).

**Não é.** Alternativas avaliadas:

| Opção | Veredito |
|---|---|
| (i) ack síncrono no IPC | **Rejeitada.** Acopla o spawn do filho à vivacidade do daemon — exatamente o que fail-open evita |
| (ii) chamador computa os IDs e os envia | **Aceita.** O wrapper já paga custo de fork; computa localmente, injeta `TRACEPARENT` no filho *antes* do spawn, notifica o daemon depois. Sem round-trip, hot-path intocado |
| (iii) terceiro replica o SHA-256 por fora | Subsumida por (ii); documentar o algoritmo basta para quem não usa nosso CLI |

Consequência: o daemon passa a **aceitar IDs externos** em vez de sempre derivá-los. Pegadinha registrada: a fórmula atual de `derive_span_id` (`trace_id.rs:34-67`) inclui um `salt` e a tupla `(conversation_id, step_idx, event, tool_name)`. Duas sub-chamadas com o mesmo tool no mesmo step **colidem** se o salt não as distinguir. Quem gera IDs externamente é responsável pela unicidade.

### 6.4 Cardinalidade

`plan_id`, `step_id`, `decision_id`, `session_id` são alta cardinalidade por natureza. Como atributos de **span** isso é normal e o ClickHouse lida bem. Como labels de **métrica**, cada valor único vira uma série persistente — explosão de storage e query.

Defesa **por tipo, não por validação em runtime**: construtores de métrica tomam campos tipados fixos e enums fechados (`ToolArchetype`, `CapabilityKind`), nunca `String` nem mapa genérico. `quota.rs` já segue esse padrão; não afrouxar.

### 6.5 Restrições do proxy stdio (L4)

- Passthrough byte-exato quando a instrumentação falha. Um erro de parse de JSON-RPC **nunca** pode corromper o stream.
- Preservar framing do transporte stdio, incluindo mensagens que não sejam requests (`notifications`, responses, batches).
- `_meta` pode já existir no request: fazer merge, não substituir. Não sobrescrever `traceparent` já presente — o cliente upstream tem precedência.
- Injeção apenas em `params` de requests e notifications, conforme SEP-414. Não em responses.
- Morte do server filho propaga exit code; sinais repassados ao grupo de processos.

---

## 7. O que fica explicitamente fora

| Fora de escopo | Motivo |
|---|---|
| Persistência, spool em disco, banco | O produto é in-memory + export imediato. A camada de análise (dataset, ECE, calibração) pertence a quem consome, não ao bridge |
| Vocabulário de domínio em `semconv.rs` | `loop.*`, `ci.*`, `eval.*` são namespaces irmãos de terceiros. Nunca aliases nem merges com `gen_ai.*`/`agent.*` |
| API de spans aninhados client-side | Seria virar um SDK OTel completo. O wrapper abre exatamente um span por invocação |
| IDE extension como alvo | Processo longo com múltiplos eventos internos e sem boundary de requisição natural. Exigiria API embarcável — a recusa que mantém o produto viável em 145 KB |
| Instrumentar o interior de MCP servers | Já resolvido por OpenInference e pelos SDKs oficiais. Nosso papel é fazer o contexto do turno chegar lá |
| Histograma | Não está fora, mas é o último movimento. Valor da tese se prova com spans; agregação/buckets é o maior mecanismo novo e pode esperar |

---

## 8. Pontos em aberto

Nenhum destes bloqueia o L1. Registrados para decisão no momento certo.

| # | Questão | Quando decidir |
|---|---|---|
| A1 | Overhead medido do hop de processo do `mcp-proxy` no caminho stdio. Precisa de número antes de recomendar uso amplo | Antes do L4 sair de experimental |
| A2 | Política de desligamento do proxy quando harnesses passarem a injetar `_meta` nativamente | Quando o primeiro harness suportar |
| A3 | Versionamento do contrato de wire (L2): SemVer próprio ou acoplado à versão do produto | No desenho do L2 |
| A4 | `derive_trace_id(conversation_id)` → renomear parâmetro para `correlation_id`. Dívida de nomenclatura, sem mudança de wire format | Oportunisticamente, com alias |
| A5 | ~~Paridade de features entre clientes Python e TypeScript a cada mudança de protocolo~~ — **resolvido**: fixture de conformidade compartilhada executada pelos três clientes no CI. Paridade é gate determinístico, não vigilância. Ver [`execution-plan.md`](execution-plan.md) § L3 | fechado |
| A6 | Se `mcp.*` mudar de forma incompatível enquanto está em Development, qual o procedimento de migração dado o invariante de retrocompat | Antes do primeiro release que emita `mcp.*` |

---

## 9. Referências

- SEP-414: Document OpenTelemetry Trace Context Propagation Conventions — status Final. `modelcontextprotocol/modelcontextprotocol`, `docs/seps/414-request-meta.mdx`, PR #414.
- Semantic conventions for Model Context Protocol — status Development. `open-telemetry/semantic-conventions-genai`, `docs/gen-ai/mcp.md`.
- W3C Trace Context — https://www.w3.org/TR/trace-context/
- W3C Baggage — https://www.w3.org/TR/baggage/
- SEP-1788 (reserved keys em `_meta`, em revisão) e SEP-2028 (`_meta` → headers HTTP).
- anthropics/claude-code#76391 — pedido de propagação de contexto para MCP via `_meta`.
- `AGENTS.md` §3.2 (semconv), §3.4 (propagação W3C, egress não implementado), §4 (gates).
