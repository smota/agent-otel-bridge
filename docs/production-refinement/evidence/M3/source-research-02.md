# M3 — pesquisa de código, tentativa 02

Estado: pesquisa de código concluída em dez batches e recebida pelo coordenador; M3 global ainda incompleto.
Fallback solicitado: gpt-6.1-sol medium, Codex subagente m3_source_research. Sem alterações de fonte, runtime ou implantação.

## Identidades verificadas pelo executor

- SigNoz v0.144.0: 47dd1fabf3105ce05ff5c3cbec463f88869adf9f.
- Collector v0.144.11: a52dc5704fd710120ae178c9d16dfa667d232b67.
- Leitura inicial de collector main foi substituída pela tag correspondente. Cache LRU visto em main não existe na tag analisada e foi excluído das conclusões.

## Achados confirmados no recibo

- Mapper prioriza atributo canônico já presente; fallback escolhe primeira origem correspondente. Operações move removem chaves de origem; não presumir preservação de todos os atributos.
- Pricing remove atributos reservados de custo; não calcula sem modelo/regra ou para todos os tokens zero. Tokens ausentes/não numéricos são lidos como zero. Modos de cache subtract/additive/unknown têm comportamentos distintos.
- Não há deduplicação de spans nos processors inspecionados.
- Agregação AI soma tokens/custos por span; total_tokens=input+output, cache separado. Pressupõe uso de tokens em spans LLM. Query em distributed_signoz_index_v3 agrupa por trace_id; totais da consulta são recortados por tempo/filtro e diferem de enriquecimento de trace completo.
- Testes foram lidos, não executados.
- Licenças lidas: código raiz SigNoz MIT, exceções ee/cmd enterprise; collector AGPLv3. Tratar como identificação de arquivos de licença, não parecer jurídico ou autorização para copiar implementação.

M3 global continua in_progress: ainda faltam runbook da estação, backup/restore dimensionados, diffs de configuração e verificações do ensaio.

## Mapping: fluxo e limites

O gerador ordena fontes habilitadas por prioridade decrescente. O processor preserva destino já existente, mesmo vazio/inadequado; caso contrário usa a primeira origem existente. Não reconcilia valores conflitantes nem valida o conteúdo do destino. Gates de grupo usam substring no nome dos atributos, não classificação semântica de spans.

Defaults incluem modelo/provedor, tokens, cache, conversa, agente e ferramenta. gen_ai.system pode alimentar gen_ai.provider.name; não deve ser preenchido artificialmente com agent-otel-bridge. A maioria dos mappings copia; mensagens podem mover. Move remove somente a origem selecionada quando o destino não existia; outras origens permanecem.

Fontes fixadas:

- [Gerador](https://github.com/SigNoz/signoz/blob/47dd1fabf3105ce05ff5c3cbec463f88869adf9f/pkg/types/spantypes/spanmapperprocessor.go).
- [Processor](https://github.com/SigNoz/signoz-otel-collector/blob/a52dc5704fd710120ae178c9d16dfa667d232b67/processor/signozspanmapperprocessor/processor.go).
- [LLM defaults](https://github.com/SigNoz/signoz/blob/47dd1fabf3105ce05ff5c3cbec463f88869adf9f/pkg/modules/spanmapper/implspanmapper/fs/definitions/gen_ai.llm.json), [agent](https://github.com/SigNoz/signoz/blob/47dd1fabf3105ce05ff5c3cbec463f88869adf9f/pkg/modules/spanmapper/implspanmapper/fs/definitions/gen_ai.agent.json), [tool](https://github.com/SigNoz/signoz/blob/47dd1fabf3105ce05ff5c3cbec463f88869adf9f/pkg/modules/spanmapper/implspanmapper/fs/definitions/gen_ai.tool.json).

## Pricing: o que é calculado

Regra é selecionada por glob ordenado do modelo, primeira correspondência; provedor não participa desse matching. Unidade de preço é por milhão de tokens, não por mil. Outputs usam namespace proprietário signoz.gen_ai.usage.*.cost.

- Sem modelo, sem regra ou quatro contagens zero: nenhum custo calculado.
- Tokens ausentes/não numéricos viram zero internamente; strings numéricas não são convertidas.
- subtract: input regular=max(input-cache_read,0); cache read/output cobrados separadamente; cache write não cobrado.
- additive: input/output/cache read/cache write calculados separadamente.
- Cache mode ausente/desconhecido: só input/output entram.
- Quando há cálculo, buckets configurados são escritos inclusive zero.
- Custos reservados preexistentes em span e resource são removidos antes do cálculo, mesmo se depois não houver novo custo.
- Processor não deduplica replay, retry, request ou identidade de span.
- Backend exclui regra com preço input/output negativo e omite cache inválido; validação do collector rejeita preço negativo. Isso não comprova validação de tokens negativos/NaN/infinito.

Fontes: [gerador](https://github.com/SigNoz/signoz/blob/47dd1fabf3105ce05ff5c3cbec463f88869adf9f/pkg/types/llmpricingruletypes/processorconfig.go), [modelo de preço](https://github.com/SigNoz/signoz/blob/47dd1fabf3105ce05ff5c3cbec463f88869adf9f/pkg/types/llmpricingruletypes/pricing.go), [config collector](https://github.com/SigNoz/signoz-otel-collector/blob/a52dc5704fd710120ae178c9d16dfa667d232b67/processor/signozllmpricingprocessor/config.go), [processor collector](https://github.com/SigNoz/signoz-otel-collector/blob/a52dc5704fd710120ae178c9d16dfa667d232b67/processor/signozllmpricingprocessor/processor.go).

## Consulta e dashboard

Explorer qualifica spans por presença de modelo OU ferramenta OU agente GenAI. Overview possui filtros separados para LLM e ferramentas; não exige gen_ai.system fabricado. Soma tokens/custos por span e agrupa por trace_id em distributed_signoz_index_v3. total_tokens=input+output, sem somar cache novamente; assume tokens somente nos spans LLM. Duplicação lógica ou totais cumulativos repetidos podem inflar o resultado.

Agregados de trace na consulta são recortados pela janela temporal e filtro de spans. Enriquecimento da lista de traces considera o trace completo; colunas aparentemente equivalentes podem ter escopos diferentes. O builder rejeita mistura de agregados de domínio span e trace na mesma query.

Fontes: [keys/gate](https://github.com/SigNoz/signoz/blob/47dd1fabf3105ce05ff5c3cbec463f88869adf9f/pkg/types/aiobservabilitytypes/keys.go), [AI scope](https://github.com/SigNoz/signoz/blob/47dd1fabf3105ce05ff5c3cbec463f88869adf9f/pkg/statementbuilder/aistatementbuilder/statement_builder.go), [trace aggregation](https://github.com/SigNoz/signoz/blob/47dd1fabf3105ce05ff5c3cbec463f88869adf9f/pkg/statementbuilder/scopedtracesstatementbuilder/trace_aggregation.go), [schema](https://github.com/SigNoz/signoz/blob/47dd1fabf3105ce05ff5c3cbec463f88869adf9f/pkg/telemetryschema/tracestelemetryschema/const.go), [overview](https://github.com/SigNoz/signoz/blob/47dd1fabf3105ce05ff5c3cbec463f88869adf9f/pkg/modules/dashboard/impldashboard/fs/definitions/ai-o11y-overview.json).

## Propostas para revisão M4

| Decisão proposta | Aplicação ao bridge |
|---|---|
| Adotar | Atributos GenAI quando houver observação real; identidade/origem e proveniência do mapping explícitas. |
| Adaptar | Operação copy para mensagens somente quando política de conteúdo permitir retenção; não habilitar conteúdo privado para preencher dashboard. |
| Adaptar | Custo downstream com regra/cache mode e origem declarados; para subscrições, somente comparador hipotético por tabela API, nunca despesa faturada ou economia comprovada. |
| Rejeitar | Provider inventado, desconhecidos preenchidos com zero, soma de cumulativos de sessão como requests, dependência de signoz.* no domínio. |
| Definir | Uma observação autoritativa de tokens por request e escopo explícito por agregado; não presumir deduplicação upstream. |

O desenho deve aprender os mecanismos sem copiar código nem criar dependência de execução do collector proprietário para funcionar. Licenças identificadas: [SigNoz](https://github.com/SigNoz/signoz/blob/47dd1fabf3105ce05ff5c3cbec463f88869adf9f/LICENSE), [collector](https://github.com/SigNoz/signoz-otel-collector/blob/a52dc5704fd710120ae178c9d16dfa667d232b67/LICENSE). Entitlements de cada feature implantada não foram auditados.

## Testes e limites

Testes fonte lidos, não executados: [mapper](https://github.com/SigNoz/signoz-otel-collector/blob/a52dc5704fd710120ae178c9d16dfa667d232b67/processor/signozspanmapperprocessor/processor_test.go), [pricing](https://github.com/SigNoz/signoz-otel-collector/blob/a52dc5704fd710120ae178c9d16dfa667d232b67/processor/signozllmpricingprocessor/processor_test.go), [trace queries](https://github.com/SigNoz/signoz/blob/47dd1fabf3105ce05ff5c3cbec463f88869adf9f/pkg/statementbuilder/scopedtracesstatementbuilder/trace_aggregation_test.go).

Casos necessários no candidato: fontes concorrentes, destino canônico vazio, copy/move, tokens ausentes/zero/string, cache modes, globs sobrepostos, requests duplicados, cumulativos em parents e janela parcial versus trace completo. Configuração/ordem de pipeline e aplicação OpAMP exigem ensaio na implantação real.

Não verificados: encoding do exportador, deduplicação na ingestão ClickHouse, reconciliação de billing por assinatura e semântica completa de null no armazenamento. A descoberta limitada é suficiente para orientar M4, não para certificar compatibilidade ou fechar M3.
