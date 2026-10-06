# M2 — inicialização isolada e hook nativo, tentativa 02

Data: 2026-10-06, 00:26–00:34 UTC. Estado: unidade diagnóstica concluída; M2 permanece **partial**. Base do checkout: 67ef649. Sem alteração de código do produto, instalação, registro de startup ou configurações de hooks.

## Resultado

A instalação ativa estava sem processo e sem pipe do bridge, enquanto o Collector Windows recebia telemetria nativa Codex. Em ambiente isolado, o mesmo binário ativo iniciou tanto em foreground quanto pelo launcher `start`, e o hook nativo entregou eventos sintéticos dos quatro clientes a um receptor OTLP independente do SigNoz.

Isso demonstra funcionamento desses caminhos nas condições testadas. Não reproduziu falha permanente de inicialização nem explica por que o processo desapareceu no ambiente normal. Não equivale a teste de hooks disparados pelos quatro harnesses reais.

## Identidade e estado observado

- Bridge ativo: 0.5.2, SHA256 `24C73837129C674D80DF727B85D04915952FC9C73B9BFFB451E6A531497C6E9D`.
- Hook ativo: SHA256 `E776F732E2773F7AEF1A70C86F8B6057824AA8070082B902585876690E05E030`.
- Divergência do manifesto anterior permanece; origem exata do build não determinada.
- Candidato atual: SHA256 `4DD1465A4BD29F79660B948AED9715556AEA449C09976A5C817A07F4D8A7C30D`. Foi recompilado nos checks anteriores; não corresponde mais ao hash ativo e não foi instalado nem testado nesta unidade.
- Collector Windows `otelcol` Running, PID7012. Boot observado: 2026-10-02 15:18:24 horário local.
- HKCU Run mantém o caminho canônico com `start`; entrada AgentOtelBridge ausente em StartupApproved/Run. Ausência dessa entrada não prova bloqueio nem execução no login.
- ClickHouse, janela de uma hora: `codex-app-server` 76626 spans e `codex-exec-server` 24; zero linhas de outros serviços no agregado e nenhum atributo `agent.hook.event`. SELECT igual ao recibo anterior, trocando 24 HOUR por 1 HOUR; limites15s/3M linhas. Último timestamp Codex app: 2026-10-06 00:29:11.817673900 UTC. Contagens não medem produtividade.

## Testes e evidências

| Teste | Observação | Limite |
|---|---|---|
| Foreground `daemon` | PID59136 confirmou ownership do pipe privado;4 frames sintéticos,4 spans,1 requisição de traces; accepted4/rejected0/unknown0/shutdown_dropped0; frameFF e exit0 | Ingresso direto, não hook nem harness |
| Launcher `start` | PID42932 lançou servidor PID57224 pertencente ao job de teste; stdout informou pipe pronto | Exit0 registrado é do launcher; código de saída do filho não foi capturado |
| Hook nativo | Quatro invocações PreToolUse --client antigravity/claude/codex/grok: exit0 e JSON;4 spans recebidos | Payload sintético comum; não reproduz os schemas completos ou lifecycle de cada harness |
| Atribuição de agente | Decodificação protobuf confirma antigravity,claude-code,codex,grok, um span por cliente | Não mede duplicação lógica de sessões reais |
| Limpeza | Nenhum pipe depois do shutdown/fechamento do job; reconsulta de processos não encontrou bridge | Sem instalação ou daemon permanente deixado na estação |

Artefatos versionáveis em [diagnosis-02](diagnosis-02/): receitas Python, stdout/stderr, recibos com ambiente mínimo, traces protobuf sintéticos e projeções JSON decodificadas. `sha256.json` registra identidade dos arquivos; os recibos originais também mencionam métricas temporárias não necessárias à conclusão de traces. Não dependemos da existência do diretório TEMP para estes resultados.

Os testes usaram ambiente explícito, diretórios vazios, pipe UUID e receptor `127.0.0.1` em porta efêmera, com job Windows kill-on-close e deadline20s. Processo criado suspenso antes de associar ao job. Ownership verificado antes do envio; sem cliente externo aceito. Uma amostra TCP não encontrou conexões no instante consultado; ela **não comprova ausência de outras conexões durante toda a execução**. Home temporário não é sandbox do token Windows. Fonte e strings deram suporte ao desenho; somente comportamento observado sustenta as conclusões.

## Defeito factual encontrado: provedor ausente vira Google

Os oito spans dos dois testes, enviados sem campo de modelo, receberam `gen_ai.provider.name=google` e `gen_ai.system=google`, inclusive Claude, Codex e Grok. O agente foi identificado corretamente.

Corroboração na fonte atual: `crates/agent-otel-core/src/otlp.rs:194` usa `input.model.as_deref().map(infer_provider).unwrap_or(GEN_AI_PROVIDER_GOOGLE)`. O default fabrica certeza quando o provedor é desconhecido e pode distorcer filtros/agrupamentos de dashboards. Não demonstra sozinho a causa de ausência de spans.

Encaminhamento M4/M6: definir semântica de provedor desconhecido e preservação dos aliases; acrescentar fixture sem modelo. Não deduzir provedor pelo nome do harness, pois um harness pode executar modelos de fornecedores diferentes. Não corrigido nesta unidade de diagnóstico.

## Cobertura por cliente e trabalho restante

| Cliente | Comprovado nesta unidade | Ainda necessário |
|---|---|---|
| AGY | Hook nativo + transporte privado + span atribuído antigravity | Evento disparado pelo AGY real, comando codificado, superfícies CLI/IDE usadas |
| Claude | Hook nativo + transporte privado + span atribuído claude-code | Exportador nativo versus hook; importação por Grok e guarda de supressão |
| Codex | Telemetria nativa no backend; hook sintético entrega span atribuído codex | Hooks efetivos/trust em CLI/Desktop e correlação com telemetria nativa |
| Grok | Hook nativo + transporte privado + span atribuído grok | Hook próprio e importado no harness real, aliases e ausência de duplicação |

## Pareceres e consumo

AGY Flash low preparou a receita a partir de digest após uma tentativa bloqueada por ferramenta. Grok Build Fast revisou o isolamento; coordenador corrigiu endpoint e nomes padrão e incorporou job/ownership. AGY Flash medium executou as duas receitas, uma por despacho. Coordenador decodificou os protobufs e verificou limpeza independentemente. Nenhum Pi, API paga ou troca de conta.

O AGY resumiu "4 export requests": eram3 requisições de métricas e1 de traces contendo4 spans. A conclusão acima usa o recibo/decodificação, não esse resumo. Consumo reportado pelos clientes fica em agent-receipts.json; não converter estimativa USD do Grok em cobrança da subscrição.

## Próxima unidade

Não reinstalar para mascarar o estado. Instrumentar a observação do lifecycle de uma sessão real em ambiente de teste: daemon privado supervisionado, harness com env herdado, evento inofensivo e marcado; preservar configs e coletar só campos necessários. Começar por AGY, depois Grok (incluindo guarda Claude), Claude e Codex. Separadamente investigar ambiente normal/login e registrar evidência de saída/startup; testes com home vazio não exercitam providers/configurações reais. Não remover isolamento nem reiniciar globalmente como fallback. M2 só fecha após completar a matriz real ou aceitar explicitamente as limitações de superfície.

## Reconciliação e revisão final

Grok Build Fast revisou os resultados em [grok-results.md](diagnosis-02/grok-results.md); M2 parcial, causa da ausência histórica não comprovada, defeito de provider separado. Seu parecer repete "shutdown graceful" no teste launcher: a evidência estrita é frame de shutdown enviado, pipe removido e job fechado; exitcode do filho continua desconhecido. Sua proposta de uma sessão real é o próximo incremento, não suficiente para fechar toda a matriz de quatro clientes.

Consulta adicional de Application IDs1000/1001 desde boot2026-10-02 retornou148 eventos, nenhum com agent-otel-bridge/agent-hook na mensagem (limite200). Isso não exclui encerramento normal, término externo ou crash não registrado. Diretório canônico logs ausente. Não há evidência disponível nesta coleta para estabelecer se o processo nunca iniciou ou encerrou depois.

Hashes ativos após os testes iguais aos anteriores. Nenhum processo bridge remanescente. Milestone M2 permanece in_progress; esta unidade encerra com resultados salvos, sem tarefas auxiliares pendentes.

Validação da entrega: cargo guardrails aprovado (fmt, Clippy, testes, conformance, docs, tamanho e branch); sintaxe Python e links locais verificados;16 hashes de artefatos conferidos. Esses checks não certificam a integração ativa dos harnesses.

Arquivos versionados preservam bytes via .gitattributes local. Whitespace do script e linha vazia final de stdout do launcher normalizados antes de calcular o manifesto; textos normalizados para LF; conteúdo dos recibos e bytes protobuf preservados.
