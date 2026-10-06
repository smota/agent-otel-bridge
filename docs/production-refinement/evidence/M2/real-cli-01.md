# M2 — sessões CLI reais dos quatro clientes

Data: 2026-10-06, aproximadamente01:09–01:14 UTC. Base ddf1f1a. Unidade de testes concluída; M2 permanece **in_progress**. Coordenador: escritor único. Grok Build Fast revisou a receita; execução por clientes reais, modelos abaixo. Nenhuma instalação, alteração de hooks/trust ou reinício global.

## Resultado por cliente

| Cliente / modelo | Ação real | Spans bridge recebidos | Resultado delimitado |
|---|---|---:|---|
| AGY / gemini-3.8-flash-low | Uma view_file do marcador, step2; resposta confere | 7: PreInvocation2, PostInvocation2, PreToolUse1, PostToolUse1, Stop1 | Transporte real CLI comprovado; sessão, modelo e step correspondem. Duas chamadas ao modelo constam do stream; pares de invocation não são automaticamente duplicação |
| Grok / grok-4.7-build-fast, low | Uma read_file; OAuth; resposta confere | 4: PreToolUse1, PostToolUse1, Stop2 | Sessão e agente correspondem; dois Stop observados, causa não determinada; não houve span claude-code nesta amostra |
| Claude / haiku solicitado | Uma Read; resposta confere | 3: PreToolUse1, PostToolUse1, Stop1 | Sessão e agente correspondem; auth claude.ai/Pro. Init anuncia claude-haiku-4-5-20251001, mas modelUsage registra claude-sonnet-5-5: não certificar uso econômico de Haiku |
| Codex / gpt-6-luna, medium | Primeira tentativa recusada com400 unsupported | 0 | Nenhuma ferramenta executada; não é falha de autenticação nem teste válido de hooks |
| Codex / gpt-5.6-luna, medium | Segunda tentativa: uma leitura PowerShell, exit0 e marcador conferido | 0 | Ferramenta real executada, mas daemon recebeu apenas HealthPing/Shutdown do supervisor; lacuna do caminho de hooks reproduzida nesta configuração |

Resultados e recibos selecionados em [real-cli](real-cli/). Cada sessão usou seu próprio daemon, pipe UUID e receiver OTLP local, com PID verificado e job Windows. Daemon e harness retornaram exit0 nas sessões válidas, pipes removidos, hashes dos hooks iguais antes/depois. Codex primeira tentativa retornou1; daemon encerrou0. Nenhum daemon de teste remanescente.

## Evidência e correlação

- AGY: sessão a7d9c366-da09-487e-88ce-534a13ef05cf; uma ferramenta view_file no índice2; modelo observado gemini-3.8-flash-low. Sete spans, um trace, IDs de spans únicos.
- Grok: sessão01a10ec3-7820-7731-a900-02bebf040b8e; tool-use ID call-97d2ff63-9d7c-48ff-b1ab-2eb6a5970ad5-0 no stream. Quatro spans, um trace; dois Stop com IDs diferentes. Apenas uma entrada própria Stop em .grok/hooks/agent-otel.json; .grok/hooks.json está vazio. Ausência de Claude nesta amostra é compatível com a guarda, mas não prova isoladamente que o hook importado foi chamado e suprimido.
- Claude: sessão6c55f3c0-c4a0-4417-ad4d-287381805cc6; tool-use ID toolu_01ScN3zxnGCumXxmbVb1bDM9 no stream. Três spans, um trace, IDs únicos.
- Os spans Grok/Claude não trouxeram gen_ai.request.model e receberam provider Google; defeito antes sintético agora observado em sessões reais.
- gen_ai.tool.call.id não apareceu nos spans dos três clientes, embora Grok/Claude exponham IDs de chamada no stream. O critério estrito de correlação por call ID não passou. Sessão + nome da ferramenta + ocorrência única (AGY também step2) permitem vincular esta amostra; não generalizar para concorrência ou repetição.
- Fonte atual model.rs:479 resolve ID somente de tool_call.id. Isso sugere lacuna de normalização, mas o stream do harness não prova o schema entregue ao hook. Capturar somente os campos necessários do payload bruto será a próxima prova antes de corrigir aliases.
- Pipeline dos três daemons: accepted7/4/3, rejected0, unknown0, shutdown_dropped0. Não inferir ausência universal de perda.

## Codex: limites da localização da falha

CLI0.154.0; sessão válida01a10ec5-c962-7c23-ae0e-9a6cfccbd13e; rollout registra gpt-5.6-luna/medium e sandbox read-only. A leitura foi executada e o resultado do marcador foi conferido. Daemon privado saudável, sem spans ou payloads inválidos; zero frames de hook admitidos.

features list mostra hooks stable=true. ~/.codex/hooks.json contém comandos PreToolUse/PostToolUse/Stop do binário canônico com --client codex; hash0a9cc7b86c46c5bb32116fc973b85bc4e84587aea96d795c410e8c2622fad48c. Existem trusted_hash para essas entradas no config.toml. Nenhum gate foi ignorado, nem confiança regravada.

codex login status: ChatGPT. codex doctor confirmou auth/config load; overall doctor terminou com advertências, não passou integralmente. Avisos incluem paridade rollout/DB e variável opcional de MCP ausente; não atribuir a eles a falha de hooks sem prova. O stderr da sessão também menciona flush do rollout/thread not found; tarefa e arquivo de rollout existem, associação causal não estabelecida.

O processo de teste herdou variáveis CODEX_* do ambiente coordenador. Portanto, não extrapolar este resultado para um lançamento independente no terminal ou para o Desktop. Também não foi capturado o ambiente do processo hook: execução/trust efetiva, quoting, herança do pipe e sandbox continuam hipóteses. A próxima unidade deve instrumentar esses pontos, comparar com ambiente mínimo do harness e preservar a configuração original. Não fazer terceira inferência nesta unidade de duas tentativas.

## Escopo e privacidade

Prova: CLI real -> hook configurado -> daemon privado -> receptor OTLP independente. Não houve encaminhamento desses spans bridge ao SigNoz, nem validação de dashboards, IDE/Desktop ou sessão normal no login. A sobrescrita OTLP do harness não prova que todo exportador nativo respeitou variáveis sobre sua configuração própria.

Modo plan para AGY/Grok/Claude e sandbox read-only para Codex. Só uma leitura por sessão válida; sem bypass de permissões. Ambiente real usado apenas pelo harness para manter subscrição/hooks; chaves API removidas do processo filho. Daemon usa home temporário vazio. Campos selecionados, recibos e hashes são persistidos; streams completos/contexto e corpos OTLP reais não entram no Git. Os registros internos normais dos próprios clientes permanecem sob responsabilidade dos clientes.

## Encaminhamento

1. Codex: investigar execução/trust/quoting/env do hook com observador delimitado; diferenciar bloqueio antes do hook de ausência de herança do pipe. Sem mudar trust/config global para forçar resultado.
2. Grok: identificar por que a mesma sessão emite dois Stop; não esconder por deduplicação de trace/span IDs, pois os IDs são distintos.
3. M4/M6: provider desconhecido não pode virar Google; contrato de correlação por tool ID depende do schema real do payload. Preservar aliases existentes.
4. Runtime: causa de daemon ausente no ambiente normal ainda não demonstrada. Não substituir por uma instalação nova durante diagnóstico.
5. Depois das correções/contratos: sessões reais até SigNoz, comparação com receptor independente e superfícies Desktop/IDE. M2 ainda não atende todo o aceite.

## Modelos e economia

Usar gemini-3.8-flash-low e grok-4.7-build-fast low nas próximas coletas. No Codex CLI, gpt-5.6-luna medium foi validado; gpt-6-luna não. Claude direto não deve usar alias haiku como garantia de modelo econômico: houve divergência entre solicitado, anunciado e uso contabilizado. Limite de duas tentativas Codex respeitado; nenhuma escalada para Astra, API paga ou compra de créditos. Valores USD reportados por clientes são estimativas, não cobrança confirmada das subscrições.

Validação da entrega: cargo guardrails aprovado; quatro recibos de sessões válidas conferidos,26 hashes e sintaxe das receitas verificados. Recibos selecionados preservam referências de traces e agregam contagens de métricas; raw streams/corpos OTLP temporários removidos após validação. Isso não fecha o aceite de M2.
