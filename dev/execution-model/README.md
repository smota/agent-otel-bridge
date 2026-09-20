# Modelo de execução: papéis, tiers e modos

Estado: generalização de uma prática já existente no repositório. Substitui, como referência ativa, as atribuições ad-hoc de [`dev/fleet-smoke/performance-coordination.md`](../fleet-smoke/performance-coordination.md), que permanece como registro histórico daquela campanha.

Este documento define **como** o trabalho é distribuído entre harnesses e modelos. Não define *o que* será construído — isso vive nos planos de cada frente, como [`dev/instrumentation-layers/`](../instrumentation-layers/).

---

## 1. A prática que está sendo generalizada

`performance-coordination.md` já continha, em forma específica, tudo o que importa:

> "Antigravity is the primary implementation and execution coordinator for this campaign; Codex independently reviews methodology, diffs, and evidence and sends correction requests. Use Gemini 3.8 Flash Low initially; escalate to Flash Medium for concrete unresolved review defects. **No Claude or silent model substitution. Deterministic measurements need no model.**"

Cinco princípios estão embutidos nesse parágrafo e são promovidos aqui a regra geral:

1. **Separação de poderes** — quem implementa não é quem revisa.
2. **Independência do revisor** — revisor em harness diferente, não subagente do implementador.
3. **Escalada sob evidência** — começa no tier baixo; sobe por defeito concreto não resolvido, nunca por precaução.
4. **Sem substituição silenciosa** — o modelo usado é declarado; trocar de modelo é decisão registrada.
5. **Trabalho determinístico não usa modelo** — `cargo test` decide melhor e mais barato que qualquer inferência.

Dois outros, também já presentes, entram junto: **posse exclusiva de recurso** ("Own all Cargo execution while working; Codex will not concurrently invoke Cargo") e **tentativas limitadas** ("at most five diagnostic attempts per scenario, no blind retries").

---

## 2. Papéis

Papéis são funções, não pessoas nem produtos. Um mesmo harness pode exercer papéis diferentes em frentes diferentes; o que não pode é exercer `driver` e `reviewer` na mesma entrega.

| Papel | Função | Produz | Não faz |
|---|---|---|---|
| `oracle` | Verificação determinística | Veredito passa/falha | Nada que exija julgamento |
| `scout` | Leitura de contexto, inventário, localização | Fatos com `file:line` | Não propõe desenho |
| `driver` | Implementa a mudança especificada | Diff + testes | Não decide desenho aberto |
| `reviewer` | Verificação independente de diff e evidência | Defeitos concretos | Não aplica correções |
| `arbiter` | Resolve divergência e decide desenho aberto | Decisão registrada | Não implementa |

Regra dura: **`driver` e `reviewer` nunca são o mesmo harness na mesma entrega.** O motivo é erro correlacionado — um implementador revisando o próprio trabalho repete a premissa que o levou ao erro, e um subagente do mesmo harness herda o mesmo contexto e as mesmas premissas.

---

## 3. Tiers de capacidade

Tiers são abstratos e independentes de fornecedor. O mapeamento para modelos concretos é um artefato mantido pelo repositório, não parte da definição.

| Tier | Nome | Natureza do trabalho |
|---|---|---|
| **T0** | `deterministic` | Compilação, testes, lint, formatação, guardrails, hashes, medições |
| **T1** | `mechanical` | Inventário, grep, leitura de contexto, edição mecânica com especificação exata |
| **T2** | `standard` | Implementação de mudança especificada, escrita de testes, refactor de rotina |
| **T3** | `judgment` | Desenho aberto, tradeoffs ambíguos, reconciliação de evidência conflitante, invariantes transversais |

### 3.1 Mapeamento — família Claude

Preços de API primeira-parte por milhão de tokens. Fonte: tabela de modelos da referência `claude-api` (cache de 2026-06-24). **Reverificar antes de usar para orçamento.**

| Tier | Modelo | ID | Input | Output | Contexto | Nota |
|---|---|---|---|---|---|---|
| T1 | Claude Haiku 4.5 | `claude-haiku-4-5` | $1,00 | $5,00 | 200K | Não aceita `effort`; usa `budget_tokens` |
| T2 | Claude Sonnet 5 | `claude-sonnet-5` | $2,00 | $10,00 | 1M | `effort` low–max |
| T3 | Claude Opus 5 | `claude-opus-5` | $5,00 | $25,00 | 1M | `effort` low–max |

Razão de custo aproximada T3:T2:T1 = **5:2:1** no input e no output.

### 3.2 Mapeamento — outros harnesses

Não preenchido. O repositório é dono desta tabela; cada linha exige verificação de tier, identificador e custo contra a fonte do fornecedor antes de entrar. `performance-coordination.md` registra o precedente para Antigravity (Gemini Flash Low como inicial, Flash Medium sob defeito concreto), que corresponde a T1 → T2.

Não inferir preço nem capacidade de um fornecedor a partir de outro.

---

## 4. A ordem correta das alavancas de custo

**Rotear por tier é a segunda alavanca, não a primeira.** Duas propriedades tornam o cascade entre modelos menos vantajoso do que parece:

1. **Caches de prompt são model-scoped.** Um cascade entre modelos perde reuso de cache entre eles. Contexto grande relido a cada troca de tier pode custar mais do que a diferença de preço por token economiza.
2. **A unidade de custo é a tarefa concluída, não a requisição.** Um tier barato que precisa de três rodadas, ou que produz um diff que o revisor rejeita, é mais caro que um tier alto que acerta na primeira.

Ordem de aplicação:

| # | Alavanca | Antes de passar para a próxima |
|---|---|---|
| 1 | Não usar modelo (T0) | Todo trabalho determinístico foi movido para comando |
| 2 | Higiene de contexto | O agente recebe o recorte necessário, não o repositório inteiro |
| 3 | `effort` dentro de um modelo | Medido que o nível mais baixo mantém a qualidade |
| 4 | Troca de tier | Medido que o tier menor conclui a tarefa, não só a requisição |
| 5 | Cross-harness | Só quando o que se compra é independência (§5) |

Corolário: antes de montar um cascade, medir o modelo mais capaz com `effort` mais baixo na mesma tarefa. É um modelo, um cache, e frequentemente mais barato por tarefa concluída.

---

## 5. Os dois modos de execução

### Modo A — cross-harnessing

Vários harnesses, uma frente de trabalho. Contextos separados, famílias de modelo distintas.

**Compra:** independência. Um revisor em outro harness não herda o contexto, as premissas nem os pontos cegos do implementador. Também arbitra quota — harnesses consomem pools de assinatura distintos, e o produto já mede isso via `agent.quota.remaining_fraction`.

**Custa:** coordenação. Nada é implícito — tudo que um harness sabe e o outro precisa tem de estar escrito. Latência de relay. Contenção de recurso (ver §6).

**Usar quando:** o modo de falha é **erro correlacionado** — um desenho errado que parece certo, uma evidência que precisa de verificação independente, um contrato que, uma vez publicado, não se corrige barato.

### Modo B — cross-model num harness só

Um harness, subagentes em tiers diferentes, contexto compartilhado.

**Compra:** economia. Fan-out de leitura em T1, síntese em T3, sem pagar tier alto por inventário.

**Custa:** reuso de cache entre os tiers (§4). E contexto compartilhado significa pontos cegos compartilhados — não substitui revisão.

**Usar quando:** o modo de falha é **custo**, não correção. Leitura ampla, inventário, edição mecânica, busca.

### 5.1 Regra de seleção

> **Cross-harness quando se precisa de independência. Cross-model quando se precisa de economia.**
>
> Nunca cross-harness só por economia — a coordenação custa mais que os tokens poupados.
> Nunca cross-model para verificação — contexto compartilhado não é independente.

Os modos compõem: o normal é Modo B dentro de cada harness e Modo A entre `driver` e `reviewer`.

---

## 6. Regras operacionais

**Posse exclusiva de recurso.** A árvore de build é recurso de escritor único. Enquanto um agente possui Cargo, nenhum outro o invoca. Vale para qualquer recurso com estado compartilhado: working tree, daemon instalado, socket, porta.

**Sem substituição silenciosa.** Toda entrega declara papel, harness e modelo usados. Trocar de modelo no meio é decisão registrada, não conveniência.

**Escalada sob evidência.** Subir de tier exige defeito concreto não resolvido no tier atual — não "parece difícil". Descer de tier exige medição, não impressão.

**Tentativas limitadas.** Número máximo de tentativas declarado antes de começar. Sem retry cego: uma falha repetida sem hipótese nova é sinal de parar e escalar, não de tentar de novo.

**Determinístico primeiro.** Antes de perguntar a um modelo, verificar se um comando responde. `cargo test`, `grep`, `wc`, um hash — todos decidem melhor e a custo zero.

**Evidência separada por superfície.** Resultado de candidato não se mistura com resultado de instalação ativa; medição local não prova visibilidade em backend. Regra herdada de `performance-coordination.md` e que continua valendo.

---

## 7. Instrumentação deste modelo

Este repositório produz exatamente o instrumento que mede se o modelo funciona. As camadas de [`dev/instrumentation-layers/`](../instrumentation-layers/) — em particular o wrapper `exec` (L1) e o contrato de domínio (L2) — permitem emitir, por unidade de trabalho: papel, harness, tier, modelo declarado, duração, custo e resultado da verificação.

Com isso a política de roteamento deixa de ser opinião e passa a ser medição: **custo por tarefa concluída, por tier, com taxa de aprovação na revisão.** Um tier que economiza por requisição e é rejeitado na revisão aparece como mais caro, que é a verdade.

Enquanto essa instrumentação não existir, as escolhas de tier deste documento são hipóteses declaradas, não resultados. Tratá-las assim.

---

## 8. Resumo operacional

| Situação | Modo | Papel/Tier |
|---|---|---|
| Descobrir onde algo está no código | B | `scout` / T1 |
| Implementar mudança já especificada | B | `driver` / T2 |
| Fixar um contrato de wire ou um invariante | — | `arbiter` / T3 |
| Verificar diff e evidência | **A** | `reviewer` / T2 em outro harness |
| Resolver divergência driver × reviewer | **A** | `arbiter` / T3 |
| Compilar, testar, medir, hashear | — | `oracle` / T0 |
