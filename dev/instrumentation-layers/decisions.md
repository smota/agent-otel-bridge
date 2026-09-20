# Decisões de desenho

Registro das decisões tomadas na discussão de arquitetura descrita em [`README.md`](README.md). Cada entrada traz as alternativas descartadas e o motivo, para que não sejam relitigadas sem fato novo.

Formato: **D-n** decisões aceitas; **R-n** posições rejeitadas que valem registro.

---

## D1 — Emitir `mcp.*` desde o início, marcado como experimental

**Contexto.** A semconv OTel para MCP (`mcp.method.name`, `mcp.session.id`, `mcp.protocol.version`, `mcp.resource.uri`) está em status **Development**. O `AGENTS.md` §3.2 exige aderência estrita a semconv e proíbe renomear ou remover constante de atributo sem alias deprecado.

**Decisão.** Emitir `mcp.*` conforme a convenção vigente, marcando explicitamente no `docs/TELEMETRY_DICTIONARY.md` que esses atributos seguem uma convenção em Development e **não** carregam a garantia de retrocompatibilidade que o resto do dicionário carrega.

**Razão.** O valor de um atributo de convenção é ser o mesmo que os outros instrumentadores emitem. Servidores instrumentados com OpenInference ou com os SDKs oficiais já emitem `mcp.*`; divergir significa que as duas metades do mesmo trace não se comparam e a correlação vira trabalho manual.

**Custo aceito.** Churn enquanto a convenção não estabiliza. Mitigação: marcação explícita no dicionário e procedimento de migração a definir (ver A6 no README).

**Alternativas descartadas:**

| Alternativa | Por que não |
|---|---|
| Manter só `capability.*` e mapear depois | Preserva o invariante sem exceções, mas nossos spans de MCP não casam com os de mais ninguém. O mapeamento tardio é dívida que cresce |
| Emitir ambos em paralelo | Duplica atributos em todo span de MCP. O produto já carrega os aliases legados `agy.*` por razão semelhante; repetir o padrão aumenta cardinalidade e custo de storage sem ganho proporcional |

---

## D2 — L4 é um proxy stdio, não um wrapper de server nem espera por suporte nativo

**Contexto.** SEP-414 (Final) fixa que `traceparent` viaja em `params._meta`. Quem deve injetá-lo é o *cliente* MCP, que vive dentro do harness. Servidores já instrumentados extraem o parent de `_meta` sozinhos.

**Decisão.** Implementar `agent-otel-bridge mcp-proxy -- <server cmd>`: um shim stdio que se interpõe entre harness e MCP server, lê o `TRACEPARENT` do ambiente, injeta no `_meta` de cada request e notification, e emite o span client-side.

**Razão.** Funciona hoje, é zero-code dos dois lados, é aderente ao padrão e independe de cada harness decidir sozinho. A alternativa de esperar suporte nativo não tem prazo e não cobre todos os harnesses.

**Custo aceito.** Um hop de processo no caminho stdio, e o proxy vira componente crítico — se ele cai, o MCP server cai junto. Por isso as restrições de passthrough em README §6.5 são obrigatórias, não recomendações.

**Alternativas descartadas:**

| Alternativa | Por que não |
|---|---|
| Depender de suporte nativo no harness (claude-code#76391) | Zero overhead, mas sem controle de prazo e cada harness decide isoladamente. Pode nunca chegar em alguns |
| Wrappear o interior do MCP server | Já resolvido por OpenInference e pelos SDKs oficiais. Duplicaríamos trabalho de terceiros e entraríamos em território de SDK |

**Nota de saída.** O proxy é ponte, não destino. Quando um harness passar a injetar `_meta` nativamente, o proxy deve poder ser desligado para aquele harness sem perda — o desenho precisa detectar `traceparent` já presente e não sobrescrever (README §6.5). Ver A2.

---

## D3 — L3 cobre Python e TypeScript, mantidos pelo projeto

**Contexto.** Clientes de instrumentação em código precisam existir na linguagem em que o alvo está escrito. MCP servers são majoritariamente Python e TypeScript; um crate Rust não os alcança.

**Decisão.** Manter clientes finos em Python e TypeScript como parte do projeto, com testes e releases próprios, além do crate Rust `agent-otel-api` para uso in-process no workspace.

**Razão.** Um contrato de wire sem implementação na linguagem do alvo não é adotado. Cobrir de fato o ecossistema MCP exige as duas linguagens.

**Custo aceito.** Dois clientes com paridade de features a manter a cada mudança de protocolo. Isso torna A5 (garantir paridade por teste, não por disciplina) uma questão de desenho do L3, não um detalhe de implementação.

**Alternativas descartadas:**

| Alternativa | Por que não |
|---|---|
| Protocolo documentado + referência só em Python | Menor manutenção, mas deixa metade do ecossistema MCP descoberta e transfere a paridade para a comunidade |
| Só crate Rust | Mantém tudo dentro dos gates atuais, mas não alcança praticamente nenhum MCP server real — o L3 perderia o propósito |

---

## D4 — Identificadores computados pelo chamador; sem ack no IPC

Detalhado em README §6.3. Resumo: o IPC permanece fire-and-forget. O wrapper computa `trace_id`/`span_id` localmente, injeta `TRACEPARENT` no filho antes do spawn e notifica o daemon depois. O daemon passa a aceitar IDs externos em vez de sempre derivá-los.

**Razão.** Um ack síncrono acoplaria o spawn do filho à vivacidade do daemon — exatamente o que o invariante fail-open existe para evitar.

---

## D5 — `HookEvent` permanece fechado; domínio trafega por `MsgType` novo

O enum de 5 eventos (`model.rs:19-26`) não ganha um sexto valor. Spans de domínio usam um `MsgType` novo no frame (slots `0x05`–`0xFE` livres; ver `frame.rs:21-27`).

**Razão.** Preserva o parser rápido do caminho de hook e a garantia de shape fixo do `AgentHookInput`, que sustenta o SLA de throughput de parsing. Um caso genérico dentro de um modelo fechado por design contamina ambos.

---

## D6 — Backlog vive em `dev/`, não em `docs/`

Segue a convenção de `dev/fleet-smoke/*-plan.md`. Documento de engenharia interno, livre para conter raciocínio, alternativas descartadas e pontos em aberto.

**Razão.** Um roadmap em `docs/` é lido por contribuidores e terceiros como compromisso de produto. Este desenho ainda tem pontos em aberto e não deve ser lido assim.

---

# Posições rejeitadas

## R1 — Implementar o vocabulário `loop.*` no core

A especificação que originou a discussão declarava `service.name=engineering-loop`. Um `service.name` distinto é, por definição, outro serviço. Absorver vocabulário de domínio de um consumidor em `semconv.rs` transformaria o bridge no backend de um gate específico e abriria precedente para `ci.*`, `deploy.*` e assim por diante.

**Substituída por:** mecanismo de prefixo reservado (README §6.2) — o produto aceita namespaces que não conhece, sem hardcodar nenhum.

## R2 — Ponto de extensão passivo como primeiro movimento

Um `MsgType` novo esperando que alguém o use não entrega valor sozinho e não justifica estabilizar um contrato. 

**Substituída por:** L1 (`exec`) como primeiro movimento — uma feature de produto que *usa* o mecanismo e prova a tese.

## R3 — Excluir MCP por ser processo de vida longa

Erro de unidade de análise. O processo é longo; a requisição JSON-RPC é efêmera. Instrumenta-se o boundary de dispatch.

**Substituída por:** D2.

## R4 — Generalizar `quota.*` e `HookEvent` para o entorno

`quota.*` é budget de provider de LLM; não existe quota de um step de build. Nomes de span derivam de `GEN_AI_OPERATION_*`. Essa é a especialização GenAI correta dentro da categoria, não um acidente de origem a ser corrigido.

**Mantido:** a especialização é valor. A tese de "processos efêmeros" aplica-se à camada de enriquecimento, não à de modelo de evento.
