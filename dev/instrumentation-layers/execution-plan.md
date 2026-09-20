# Plano de execução das camadas L1–L4

Aplica o [modelo de execução](../execution-model/README.md) ao [breakdown técnico](breakdown.md). Papéis (`scout`/`driver`/`reviewer`/`arbiter`/`oracle`), tiers (T0–T3) e modos (A = cross-harness, B = cross-model num harness) estão definidos lá e não são repetidos aqui.

Ordem: **L1 → L2 → L4 → L3**. Nenhuma camada iniciada.

---

## Princípio de alocação

O tier alto só toca trabalho em que **errar é caro e difícil de detectar**. Isso não é o mesmo que trabalho difícil.

Um contrato de wire publicado (L2) e a semântica de passthrough de um proxy (L4) qualificam: o erro sobrevive ao review, chega em produção e o custo de correção cresce com a adoção. Escrever um subcomando, um arquivo de teste ou um cliente em Python não qualifica — o erro aparece no `cargo test` ou no primeiro uso.

Consequência contraintuitiva e deliberada: **L2 recebe mais T3 que L4**, embora L4 tenha mais risco de correção. O risco do L4 é *detectável* — property tests e fuzzing encontram quebra de passthrough. O risco do L2 é uma decisão de contrato que parece correta e só se revela cara quando há adoção, quando já não dá para mudar.

---

## L1 — `exec` wrapper

**Perfil.** Especificação fechada, complexidade moderada, área de impacto pequena (só o CLI). O risco concentra-se em semântica de processo — sinais, exit codes, passthrough de stdio — que é onde moram bugs silenciosos e onde as plataformas divergem.

| # | Item | Papel | Tier | Modo |
|---|---|---|---|---|
| 1 | Inventário: como `Commands` liga subcomandos em `main.rs`, padrões de teste de processo existentes em `tests/native_context_process.rs` | `scout` | T1 | B |
| 2 | Implementar `exec.rs` conforme a sequência de 7 passos do breakdown | `driver` | T2 | B |
| 3 | Testes de exit code, sinal, passthrough, daemon ausente, `TRACEPARENT` no filho | `driver` | T2 | B |
| 4 | `cargo fmt`, `clippy -D warnings`, `test --workspace`, `doc`, `guardrails` | `oracle` | **T0** | — |
| 5 | Revisão independente do diff, com foco em semântica de sinais e exit code em Unix **e** Windows | `reviewer` | T2 | **A** |
| 6 | Divergência entre 2 e 5 | `arbiter` | T3 | **A** |

**Por que a revisão é Modo A aqui.** O implementador que escolheu `fork+wait` em vez de `execve`-replace carrega a premissa que motivou a escolha. Se ela estiver errada em alguma plataforma, ele não a questiona. Um revisor em outro harness chega sem ela.

**Item 6 é condicional.** Só existe se houver divergência concreta. Escalar por precaução viola a regra de escalada sob evidência.

---

## L2 — Contrato de ingestão

**Perfil.** A camada mais cara de errar do conjunto inteiro. Formato de wire, versionamento, prefixos reservados e limites de payload são decisões que, depois de adotadas por um cliente externo, não se corrigem sem quebrar alguém.

| # | Item | Papel | Tier | Modo |
|---|---|---|---|---|
| 1 | **Fixar o contrato**: layout do payload, esquema de versionamento (A3), lista de prefixos reservados, teto de tamanho e sua relação com o orçamento do micro-batch | `arbiter` | **T3** | — |
| 2 | Revisão do contrato **antes** de qualquer código | `reviewer` | T3 | **A** |
| 3 | Implementar `MsgType`, encode/decode, validação de prefixo, `build_span_from_domain` | `driver` | T2 | B |
| 4 | Testes: roundtrip, rejeição por prefixo, nome fora do limite, mapeamento de status, IDs externos, frame malformado | `driver` | T2 | B |
| 5 | Gates | `oracle` | **T0** | — |
| 6 | Revisão do diff contra o contrato fixado em 1 | `reviewer` | T2 | **A** |

**A inversão que importa:** T3 entra **antes** do código, não depois. Revisar um contrato já implementado é caro — o custo afundado empurra para aceitar o que está lá. Revisar o contrato em prosa, antes de existir implementação, é barato e é o único momento em que mudá-lo não custa nada.

**Item 2 é a única revisão T3-contra-T3 do plano.** Justifica-se porque o artefato revisado é uma decisão, não um diff.

---

## L4 — `mcp-proxy`

**Perfil.** Especificação **externa e fechada** (SEP-414, status Final), o que remove o julgamento de desenho. Sobra risco de correção concentrado em passthrough e em casos de borda do JSON-RPC. Risco alto, mas detectável por teste — logo, mais T0 e menos T3.

| # | Item | Papel | Tier | Modo |
|---|---|---|---|---|
| 1 | Enumerar pontos de conformidade da SEP-414 e todas as formas de mensagem JSON-RPC a preservar (requests, notifications, responses, batches, erros) | `scout` | T1 | B |
| 2 | Implementar o shim: leitura de `TRACEPARENT`, merge em `_meta`, span client-side com `mcp.*` | `driver` | T2 | B |
| 3 | **Property tests de passthrough**: para qualquer entrada, a saída é byte-idêntica quando a instrumentação é pulada | `driver` + `oracle` | T2 → **T0** | B |
| 4 | Testes de conformidade: `_meta` preexistente sofre merge; `traceparent` preexistente não é sobrescrito; injeção só em requests e notifications | `driver` | T2 | B |
| 5 | Gates | `oracle` | **T0** | — |
| 6 | Revisão **adversarial**: tarefa explícita de quebrar o passthrough, não de aprovar o diff | `reviewer` | T2 | **A** |

**Item 3 é a conversão-chave do plano.** "O proxy não corrompe o stream" é uma propriedade verificável, não uma opinião. Uma vez escrito o gerador, a verificação é T0 e roda para sempre a custo zero. Escrever o property test em T2 é o melhor uso de tier deste documento: paga-se uma vez e remove um modelo do caminho permanentemente.

**Item 6 tem enquadramento diferente dos outros reviewers.** Um revisor com a tarefa "aprove ou aponte defeitos" procura confirmação. Um revisor com a tarefa "quebre isto" procura contraexemplo. Para passthrough, o segundo enquadramento encontra o que o primeiro não encontra.

---

## L3 — Clientes de instrumentação

**Perfil.** Volume alto, julgamento baixo por unidade, duas linguagens a manter em paridade. O problema real não é escrever os clientes — é impedir que divirjam ao longo do tempo.

| # | Item | Papel | Tier | Modo |
|---|---|---|---|---|
| 1 | **Fixture de conformidade compartilhada**: casos do contrato L2 em formato neutro de linguagem, que qualquer cliente executa | `arbiter` | **T3** | — |
| 2 | Cliente Python contra a fixture | `driver` | T2 | B |
| 3 | Cliente TypeScript contra a fixture | `driver` | T2 | B |
| 4 | Crate `agent-otel-api` (Rust) | `driver` | T2 | B |
| 5 | Gate de CI executando a fixture nos três clientes | `oracle` | **T0** | — |
| 6 | Revisão de paridade semântica onde a fixture não alcança (fail-open sob daemon morto, comportamento em erro de socket) | `reviewer` | T2 | **A** |

**Resolução do ponto em aberto A5.** A pergunta registrada no backlog era como garantir paridade entre Python e TypeScript "por teste e não por disciplina". A resposta é o item 1: **paridade é problema T0, não problema de modelo.** Uma fixture compartilhada que os três clientes executam no CI transforma uma questão de vigilância contínua num gate determinístico.

O item 1 é T3 porque desenhar a fixture *é* redesenhar o contrato sob outra forma — os casos que ela não cobrir são exatamente onde os clientes vão divergir.

---

## Recursos exclusivos

Vale por toda a sequência, em Modo A especialmente.

| Recurso | Regra |
|---|---|
| Cargo / árvore de build | Escritor único. Quem implementa possui; o revisor não invoca Cargo em paralelo |
| Working tree e branch | Um `driver` por vez. Revisor lê diff, não edita |
| Daemon instalado, socket, named pipe | Nenhuma camada mexe na instalação ativa. Testes usam endpoint próprio e isolado |
| Gate de CI | T0 é a autoridade final. Nenhum papel declara verde sem o gate |

---

## Declaração obrigatória por entrega

Sem substituição silenciosa. Toda entrega registra:

```
camada:   L1 | L2 | L3 | L4
item:     <número da tabela>
papel:    scout | driver | reviewer | arbiter | oracle
harness:  <qual>
tier:     T0 | T1 | T2 | T3
modelo:   <identificador exato>
escalada: não | <defeito concreto que a motivou>
gates:    <resultado de cada gate T0>
```

O campo `escalada` é o que mantém a regra honesta: subir de tier sem nomear o defeito que a motivou é o modo silencioso de o custo crescer.

---

## Resumo

| Camada | T3 | T2 | T1 | T0 | Modo A em |
|---|---|---|---|---|---|
| L1 | condicional | impl + revisão | inventário | gates | revisão |
| L2 | **contrato, antes do código** | impl + revisão de diff | — | gates | revisão de contrato **e** de diff |
| L4 | — | impl + revisão adversarial | inventário de conformidade | gates + **property tests** | revisão adversarial |
| L3 | **fixture de paridade** | três clientes | — | gates + **fixture no CI** | revisão de paridade semântica |

Duas leituras do quadro. Primeira: T3 aparece três vezes, sempre **antes** do código e sempre sobre um artefato de contrato — nunca para escrever implementação. Segunda: as duas conversões de T2 para T0 (property tests no L4, fixture no L3) são os melhores investimentos do plano, porque removem um modelo do caminho permanentemente.
