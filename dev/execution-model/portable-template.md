# Modelo de execução multi-harness e multi-modelo — template portável

Versão genérica do [modelo de execução](README.md), sem referências a este repositório. Destina-se a ser copiada para o `CLAUDE.md`, `AGENTS.md` ou guia de contribuição de outro projeto.

Ao adotar, preencher as quatro lacunas marcadas **`[PREENCHER]`**. Um template com essas lacunas em branco não funciona — elas são o que liga o modelo abstrato ao projeto concreto.

---

## 0. Antes de adotar: este projeto precisa disto?

O modelo tem custo de coordenação real. Ele se paga quando pelo menos uma destas condições vale:

- O trabalho contém **decisões irreversíveis** — formato de dado publicado, contrato de API, esquema de banco, protocolo de wire.
- O trabalho **atravessa sessões** e o contexto precisa sobreviver a quem o executa.
- A **correção excede o que os testes capturam** — concorrência, semântica de processo, compatibilidade entre plataformas.
- Há **evidência a verificar**, não só código a revisar — medições, benchmarks, resultados que podem estar certos pelo motivo errado.

Se nenhuma vale — script de um arquivo, protótipo descartável, mudança mecânica — não adote. A separação de papéis vai custar mais do que entrega.

---

## 1. Princípios

1. **Separação de poderes.** Quem implementa não é quem revisa.
2. **Independência do revisor.** Revisor em harness diferente, não subagente de quem implementou.
3. **Escalada sob evidência.** Começa no tier baixo. Sobe por defeito concreto não resolvido, nunca por precaução.
4. **Sem substituição silenciosa.** O modelo usado é declarado. Trocar é decisão registrada.
5. **Trabalho determinístico não usa modelo.** Um comando decide melhor e a custo zero.

---

## 2. Papéis

| Papel | Função | Produz | Não faz |
|---|---|---|---|
| `oracle` | Verificação determinística | Veredito passa/falha | Nada que exija julgamento |
| `scout` | Leitura de contexto, inventário | Fatos com `arquivo:linha` | Não propõe desenho |
| `driver` | Implementa o que foi especificado | Diff + testes | Não decide desenho aberto |
| `reviewer` | Verificação independente | Defeitos concretos | Não aplica correções |
| `arbiter` | Resolve divergência, decide desenho aberto | Decisão registrada | Não implementa |

**Regra dura:** `driver` e `reviewer` nunca são o mesmo harness na mesma entrega. O motivo é erro correlacionado — quem implementou revisando o próprio trabalho repete a premissa que causou o erro, e um subagente do mesmo harness herda contexto e premissas junto.

---

## 3. Tiers de capacidade

Abstratos e independentes de fornecedor.

| Tier | Nome | Natureza do trabalho |
|---|---|---|
| **T0** | `deterministic` | Compilação, testes, lint, formatação, medições, hashes |
| **T1** | `mechanical` | Inventário, busca, leitura de contexto, edição com especificação exata |
| **T2** | `standard` | Implementação especificada, escrita de testes, refactor de rotina |
| **T3** | `judgment` | Desenho aberto, tradeoffs ambíguos, evidência conflitante, invariantes transversais |

### **`[PREENCHER 1]`** — Mapeamento tier → modelo

Preencher com os modelos disponíveis no projeto. **Verificar preço e identificador na fonte do fornecedor antes de usar para orçamento** — não inferir de um fornecedor para outro, e não confiar em valores memorizados.

Exemplo com a família Claude (preços de API primeira-parte por milhão de tokens, cache de 2026-06-24 — reverificar):

| Tier | Modelo | ID | Input | Output | Contexto |
|---|---|---|---|---|---|
| T1 | Claude Haiku 4.5 | `claude-haiku-4-5` | $1,00 | $5,00 | 200K |
| T2 | Claude Sonnet 5 | `claude-sonnet-5` | $2,00 | $10,00 | 1M |
| T3 | Claude Opus 5 | `claude-opus-5` | $5,00 | $25,00 | 1M |

Razão de custo T3:T2:T1 ≈ **5:2:1**.

---

## 4. A ordem das alavancas de custo

**Rotear por tier é a segunda alavanca, não a primeira.** Duas propriedades derrubam a intuição:

1. **Caches de prompt são model-scoped.** Um cascade entre modelos perde reuso de cache entre eles. Contexto grande relido a cada troca de tier pode custar mais do que a diferença de preço por token economiza.
2. **A unidade de custo é a tarefa concluída, não a requisição.** Um tier barato que precisa de três rodadas, ou cujo diff o revisor rejeita, é mais caro que um tier alto que acerta de primeira.

| # | Alavanca | Antes de passar para a próxima |
|---|---|---|
| 1 | Não usar modelo (T0) | Todo trabalho determinístico virou comando |
| 2 | Higiene de contexto | O agente recebe o recorte necessário, não o projeto inteiro |
| 3 | `effort` dentro de um modelo | Medido que o nível mais baixo mantém a qualidade |
| 4 | Troca de tier | Medido que o tier menor **conclui a tarefa**, não só a requisição |
| 5 | Cross-harness | Só quando o que se compra é independência |

Corolário: antes de montar um cascade, medir o modelo mais capaz com `effort` mais baixo na mesma tarefa. Um modelo, um cache, frequentemente mais barato por tarefa concluída.

---

## 5. Os dois modos

**Modo A — cross-harnessing.** Vários harnesses, uma frente. Contextos separados, famílias de modelo distintas.

- *Compra:* independência. O revisor não herda contexto, premissas nem pontos cegos de quem implementou. Também arbitra quota, quando os harnesses consomem pools de assinatura distintos.
- *Custa:* coordenação — nada é implícito, tudo que um sabe e o outro precisa tem de estar escrito. Latência de relay. Contenção de recurso.
- *Usar quando:* o modo de falha é **erro correlacionado** — desenho errado que parece certo, evidência que precisa de verificação independente, decisão que depois de publicada não se corrige barato.

**Modo B — cross-model num harness só.** Subagentes em tiers diferentes, contexto compartilhado.

- *Compra:* economia. Fan-out de leitura em T1, síntese em T3.
- *Custa:* reuso de cache entre tiers. E contexto compartilhado significa pontos cegos compartilhados.
- *Usar quando:* o modo de falha é **custo**, não correção.

### Regra de seleção

> **Cross-harness quando se precisa de independência. Cross-model quando se precisa de economia.**
>
> Nunca cross-harness só por economia — a coordenação custa mais que os tokens poupados.
> Nunca cross-model para verificação — contexto compartilhado não é independente.

Os modos compõem: o normal é Modo B dentro de cada harness e Modo A entre `driver` e `reviewer`.

---

## 6. Princípio de alocação

**Tier alto só toca trabalho em que errar é caro E difícil de detectar.** Isso não é o mesmo que trabalho difícil.

Um contrato publicado qualifica: o erro sobrevive ao review, chega em produção, e o custo de correção cresce com a adoção. Uma implementação complexa mas coberta por testes não qualifica — o erro aparece no gate.

Consequência prática, frequentemente contraintuitiva: uma peça de **alto risco mas risco detectável** merece menos T3 que uma peça de **risco menor porém invisível**.

Dois padrões que se repetem e valem ser procurados ativamente:

**T3 antes do código, nunca depois.** Para qualquer artefato de contrato — formato, esquema, protocolo, interface pública — fixar e revisar em prosa **antes** de existir implementação. É o único momento em que mudá-lo custa zero. Revisar contrato já implementado é caro porque o custo afundado empurra para aceitar o que está lá.

**Converter T2 em T0.** Quando uma propriedade pode virar teste — "a saída é byte-idêntica", "os dois clientes concordam" — escrever esse teste é o melhor uso de tier que existe: paga-se um modelo uma vez e remove-se o modelo do caminho permanentemente. Procurar essas conversões antes de aceitar revisão recorrente como custo fixo.

---

## 7. Regras operacionais

**Posse exclusiva de recurso.** Recursos com estado compartilhado têm escritor único. Enquanto um agente possui, nenhum outro invoca.

> **`[PREENCHER 2]`** — listar os recursos exclusivos do projeto: ferramenta de build, working tree, serviço instalado, portas, endpoints de teste.

**Autoridade do gate.** Nenhum papel declara verde sem o gate determinístico.

> **`[PREENCHER 3]`** — listar os comandos que são autoridade final: build, lint, testes, formatação, checagens próprias.

**Harnesses disponíveis.**

> **`[PREENCHER 4]`** — quais harnesses existem neste ambiente e qual é o par `driver`/`reviewer` padrão.

**Escalada sob evidência.** Subir de tier exige defeito concreto não resolvido — não "parece difícil". Descer exige medição, não impressão.

**Tentativas limitadas.** Máximo declarado antes de começar. Falha repetida sem hipótese nova é sinal de parar e escalar, não de tentar de novo.

**Evidência separada por superfície.** Resultado local não prova comportamento em produção. Resultado de candidato não se mistura com o de instalação ativa.

---

## 8. Declaração obrigatória por entrega

```
frente:   <identificador do trabalho>
item:     <número ou nome>
papel:    scout | driver | reviewer | arbiter | oracle
harness:  <qual>
tier:     T0 | T1 | T2 | T3
modelo:   <identificador exato>
escalada: não | <defeito concreto que a motivou>
gates:    <resultado de cada gate T0>
```

O campo `escalada` é o que mantém a regra honesta: subir de tier sem nomear o defeito é o modo silencioso de o custo crescer.

---

## 9. Honestidade sobre o estado

Enquanto o projeto não medir **custo por tarefa concluída por tier, com taxa de aprovação na revisão**, as escolhas de tier são hipóteses declaradas, não resultados. Tratá-las assim — e revisá-las quando houver medição.

Um tier que economiza por requisição e é rejeitado na revisão aparece, na medição correta, como o mais caro. Essa é a única métrica que decide.
