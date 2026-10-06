# Refinamento de produção — milestones e continuidade

Criado: 2026-09-30. Revisado: 2026-10-06. Execução em andamento; posição e evidências em [CHECKPOINT.md](CHECKPOINT.md).

## Objetivo e escopo

Operação pessoal confiável no Windows com AGY, Claude, Codex e Grok; SigNoz self-hosted no WSL. Pi está fora desta rodada. Todos os clientes usam subscrição. Não usar APIs pagas, comprar créditos ou mudar de conta como fallback.

Entregar diagnóstico verificável, pesquisa do SigNoz, contratos de produto, dashboards orientados a decisões, atualização ensaiada e release validada. Manter o bridge independente do SigNoz e cumprir AGENTS.md e docs/local-runtime-contract.md.

Este plano incorpora a matriz de modelos da conversa e o parecer consultivo obtido com grok-4.7. O parecer usou um resumo de evidências, sem inspeção de código; não constitui revisão de implementação.

## Continuidade: duas fontes, responsabilidades distintas

- PLAN.md: escopo, ordem, contratos e modelos. Alterar somente quando a decisão mudar.
- CHECKPOINT.md: posição atual, resultados, trabalho parcial, próximo passo e impedimentos. É o ponto de entrada para retomada.
- evidence/<milestone>/<task>-<attempt>.md: recibos curtos e imutáveis criados conforme o trabalho ocorre. Incluir comandos, resultados, caminhos e hashes; nunca credenciais ou transcrições completas.

O checkpoint é um índice de evidências, não substitui resultados de testes. Guardar capturas necessárias à aceitação; evitar depender exclusivamente de logs temporários ou retenção do backend. Arquivos grandes ficam fora do Git com caminho e SHA-256 registrados e disponibilidade verificada na retomada.

Usar o protocolo AgentFlow: contrato de aceite -> entrega -> verificação -> decisão. Este checkout ainda não tem adoção local configurada dos quatro pilares. Estes documentos são o registro ativo da campanha, não um run nativo AgentFlow. Não manter dois registros mutáveis concorrentes. Se a campanha migrar para run nativo, declarar a nova fonte de verdade e tornar o checkpoint uma projeção com ponteiro.

Referências locais do framework: docs/run-operations.md, docs/role-collaboration.md e lib/config/harness-intelligence.mjs em C:/Users/samue/code/agentflow-sdlc. A infraestrutura run context/checkpoint/resume só será usada após configuração e verificação; não presumir que já está ativa.

## Milestones

Cada milestone pode ser concluído e retomado independentemente, respeitando suas dependências. Dividir implementação em uma alteração verificável por tarefa; não aguardar o milestone inteiro para salvar.

| ID | Entrega e tarefas | Dependências | Modelo principal / executor | Conclusão verificável |
|---|---|---|---|---|
| M0 | Plano, checkpoint e regras de continuidade | Nenhuma | Coordenador atual; manutenção futura: gemini-3.8-flash-low / AGY | Arquivos salvos, coerentes e próximos passos explícitos |
| M1 | Linha de base: hashes reais, manifestos, processos, inicialização, hooks; Collector Windows; distro, serviços, versões e volumes WSL; dashboards publicados e geradores | M0 | Concluído; revalidação delimitada: gemini-3.8-flash-low / AGY | Inventário com caminhos, horários e diferenças; estado preservado antes de reiniciar ou instalar |
| M2 | Diagnóstico por cliente e por fronteira; atividade nativa versus bridge; duplicação, correlação, entrega e consultas | M1 | Coleta: gemini-3.8-flash-low / AGY; diagnóstico: grok-4.7-build-fast / Grok; receita isolada: gemini-3.8-flash-medium / AGY | Quatro fichas de cobertura; hipóteses confirmadas/refutadas ou lacunas explícitas; causas suficientes para delimitar correções |
| M3 | Pesquisa upstream SigNoz e preparação do upgrade WSL: código, templates, convenções, migrações e backup/restauração | M1 | Fontes e runbook: gemini-3.8-flash-medium / AGY; revisão: grok-4.7-build-fast / Grok; riscos de migração: grok-4.7 / Grok | Relatório com commits/fontes, matriz adotar/adaptar/descartar e runbook com versão alvo fixada |
| M4 | Contratos de sinais e ADRs: independência OTLP, origem, deduplicação, correlação, dados ausentes, assinatura versus custo e rate limit | M2, M3 | Extração e redação: gemini-3.8-flash-medium / AGY; decisão arquitetural: grok-4.7 / Grok; revisão: gpt-6.1-sol medium / Codex | Parecer Grok, revisão independente e objeções resolvidas; contratos aceitos pelo coordenador |
| M5 | Jornadas do operador; inventário de todos os painéis; manter/corrigir/mover/fundir/retirar; esboços e consultas propostas | M4 | Inventário: gemini-3.8-flash-low / AGY; jornadas e consultas: gemini-3.8-flash-medium / AGY; revisão: grok-4.7-build-fast / Grok | Mapa pergunta -> decisão -> sinal -> próximo passo e revisão com Samuel; principal com até seis painéis essenciais |
| M6 | Correções do bridge em candidato isolado, uma fatia por defeito; fixtures e regressões; contratos de runtime preservados | M4 | Implementação: gemini-3.8-flash-medium / AGY; revisão comum: grok-4.7-build-fast / Grok; IPC/unsafe/watchdog: gpt-6.1-sol medium / Codex + revisão crítica independente | Fatias revisadas; checks obrigatórios aprovados; evidência do candidato separada da instalação ativa |
| M7 | Ensaio de atualização SigNoz em cópia isolada e ensaio de restauração; executar atualização ativa após ensaio aprovado | M3, M4 | Runbook e execução da receita ensaiada: gemini-3.8-flash-medium / AGY; parecer de recuperação: grok-4.7 / Grok; revisão crítica independente antes da operação ativa | Identidade/versionamento, ingestão e consultas comparadas; recuperação exercitada; operação ativa reconciliada |
| M8 | Implementar dashboards e separação de assinaturas; validar consultas, dados ausentes e navegação | M5, M6, M7 | Implementação: gemini-3.8-flash-medium / AGY; revisão de consultas e narrativa: grok-4.7-build-fast / Grok | Evidência por painel, inspeção visual e jornada do operador validada |
| M9 | Instalação atômica do candidato; validação real dos quatro clientes; concorrência, reinício Windows/WSL e recuperação | M6, M7, M8 | Coleta e receita de instalação aprovada: gemini-3.8-flash-medium / AGY; análise de cobertura: grok-4.7-build-fast / Grok; aceite crítico: gpt-6.1-sol medium / Codex | Matriz de aceite completa; hashes corretos; carga e limites declarados; receptor OTLP independente verificado |
| M10 | Fechar release: notas, limitações, runbook e índice final de evidências | M9 | Notas e índice: gemini-3.8-flash-low / AGY; revisão de evidências: grok-4.7-build-fast / Grok; aceite final pelo coordenador | Pacote de lançamento pronto; publicação e implantação identificadas separadamente, sem inferir que ocorreram |

M6 e M7 são mudanças separadas: nunca atualizar bridge e backend no mesmo passo operacional. Com um auxiliar por vez, executar a ordem numérica; dependências permitem retomar frentes independentes quando uma subscrição estiver indisponível.

## Tarefas obrigatórias dentro de M2

| Cliente | Verificar | Evidência final |
|---|---|---|
| AGY | Configuração efetiva, PowerShell/quoting, hooks e cada superfície usada | Evento real identificado até o backend, sem confundir teste de login com teste de hook |
| Claude | Hooks, exportadores nativos e variáveis por sinal; importações por Grok | Origem dos sinais e ausência de dupla contagem |
| Codex | Hooks efetivos/confiáveis; CLI/Desktop usados; correlação suportada pelo harness | IDs/parentesco comprovados ou limitação explícita |
| Grok | Hooks próprios/importados, aliases de payload e identificação | Uma ocorrência por evento esperado e atribuição correta |

Etiquetar as sessões de investigação para distingui-las da atividade observada. Verificar relógios, receptor OTLP, endereços e portas entre Windows e WSL. Nenhuma leitura do backend deve expor prompts ou segredos sem necessidade.

## Modelos, fallback e revisão

Política revisada em 2026-10-06: priorizar AGY Flash e Grok Build Fast. Esta é uma alocação para economizar capacidade de subscrição, não uma comparação comprovada de preço ou consumo por chamada. Medir tentativas, tempo e aceite; consumo não exposto permanece unknown. A orientação de selecionar modelos conforme a tarefa está na [documentação oficial OpenAI](https://developers.openai.com/api/docs/guides/model-selection); os IDs desta matriz dependem dos catálogos disponíveis na estação.

| Operação | Principal | Fallback da mesma faixa | Quando escalar |
|---|---|---|---|
| Inventário, extração, índice, formatação | gemini-3.8-flash-low / AGY | gpt-5.6-luna medium / Codex CLI | Extração ambígua: Flash medium, com exemplos concretos |
| Diagnóstico delimitado e revisão comum | grok-4.7-build-fast / Grok | gemini-3.8-flash-medium / AGY; Luna medium se autor for Flash | Hipótese não resolvida com evidências: Grok 4.7 em parecer curto |
| Pesquisa de código, fixtures, adaptadores e dashboards | gemini-3.8-flash-medium / AGY | grok-4.7-build-fast / Grok | Falha reproduzível persistente: gpt-6.1-sol medium / Codex |
| Arquitetura, contratos e recuperação de migração | grok-4.7 / Grok, somente parecer delimitado | gpt-6.1-sol medium / Codex, parecer provisório | Fechar M4 requer parecer Grok; executar outra tarefa se indisponível |
| Implementação crítica em IPC/unsafe/watchdog | gpt-6.1-sol medium / Codex | gemini-3.1-pro-high / AGY | Sol high somente com falha concreta ou risco não resolvido documentado |
| Revisão crítica de instalação, migração, OTLP ou concorrência | gpt-6.1-sol medium / Codex | claude-sonnet-5-5-medium / AGY | Se Sol for autor, usar Sonnet; aprofundar apenas o ponto sem evidência |

Catálogos consultados: agy models e grok models em 2026-10-06. AGY lista Flash 3.8 low/medium, Gemini 3.1 Pro high e Sonnet 5.5 medium; o antigo claude-sonnet-4-6 não foi listado e saiu do roteamento. Grok lista grok-4.7-build-fast e grok-4.7; uma segunda consulta confirmou login grok.com após resposta transitória de autenticação. Catálogo/login não comprovam execução de inferência nem capacidade disponível. Revalidar no despacho; nunca substituir silenciosamente o ID solicitado.

Verificação em sessões reais (2026-10-06): AGY gemini-3.8-flash-low e Grok grok-4.7-build-fast low executaram leituras. No Codex CLI0.154.0, gpt-6-luna retornou400 unsupported apesar de listado no cache; gpt-5.6-luna medium executou a tarefa com conta ChatGPT. Usar este último como fallback econômico do CLI enquanto a incompatibilidade persistir; o catálogo do Desktop não certifica o backend do CLI. No Claude direto, alias haiku foi apresentado como claude-haiku-4-5-20251001 no init, mas modelUsage informou claude-sonnet-5-5. Não tratar esse alias como garantia econômica; preferir AGY/Grok para tarefas comuns e registrar solicitado, anunciado e contabilizado separadamente. Claude auth status confirmou subscrição Pro/claude.ai. Nenhuma inferência via API paga nesta unidade.

Modelo e cliente são campos separados. Sonnet via Claude direto pode substituir o transporte AGY após resolver e registrar o ID exato disponível. Não tratar o alias sonnet como prova de versão.

Revisão comum usa modelos diferentes: Flash -> Grok Build Fast; Grok Build Fast -> Flash medium. Revisão crítica usa Sol medium ou Sonnet 5.5 medium, diferente do autor. Quem diagnosticou o defeito não pode ser o único revisor crítico. O coordenador registra aceite e objeções; outra sessão no mesmo modelo não cria independência.

Não despachar Sol high, Astra ou Opus por padrão de milestone. Antes de escalar, corrigir escopo, ferramentas e contrato de evidência; pesquisa baseada apenas em buscas não passa. Exigir arquivos/diffs upstream com commit e caminhos. Reutilizar pesquisa aceita de M3; atualizar somente fontes ou versões que mudaram. Falha de quota troca a subscrição disponível ou suspende a tarefa, sem justificar modelo mais exigente. Após duas tentativas, salvar diagnóstico e criar tarefa menor de escalada; não reiniciar a mesma tarefa em loop.

A revisão documental atual é feita por um escritor, sem novas consultas LLM externas. Os comandos AgentFlow collaboration classify/plan foram consultados sem parâmetros de risco e retornaram bilateral/advisory com defaults; isso não reclassifica a campanha crítica nem certifica autenticação. Antes de cada mudança técnica, classificar com risco e superfície explícitos e aplicar o contrato de revisão acima. O binding automático de cliente não substitui a preferência por AGY/Grok desta matriz.

## Subscrições e limites de trabalho

- Preflight antes de cada tarefa: cliente, modelo solicitado, autenticação e disponibilidade. Consultar saldo/reset somente onde houver fonte confiável. Desconhecido permanece desconhecido.
- Registrar a subscrição/conta usada com identificador não sensível. Não supor que trocar cliente ou modelo troca o saldo disponível. Todos os modelos Codex podem disputar o mesmo limite; AGY pode ter regras próprias por modelo.
- Uma tarefa por despacho; um auxiliar por vez; um escritor por checkout. Janela alvo de 15–30 minutos e até dez turnos por tentativa, limites operacionais indicativos onde o cliente não os impõe.
- Salvar ao terminar cada resultado verificável, antes de delegar e antes de ações externas. Não confiar em uma reserva de crédito para o último resumo.
- Até duas tentativas por tarefa. Uma falha explícita de quota: registrar e trocar para fallback disponível, sem loop de retries. Se não houver fallback, marcar waiting_capacity e avançar somente tarefa independente.
- Reset informado pelo cliente é uma observação com horário; não prometer retomada automática sem mecanismo configurado. Não criar automação nesta rodada.
- Uso e custo não observáveis ficam unknown. Subscrição não implica capacidade ilimitada nem medição por dólar.

## Protocolo de checkpoint

Antes de iniciar: salvar tarefa, responsável, modelo solicitado, checkout/base, arquivos permitidos, aceite e efeitos externos possíveis. Durante: salvar artefato ao concluir cada unidade verificável. Ao sair: atualizar resultado, modelo observado (se conhecido), arquivos alterados, testes, impedimento e próximo comando.

Estados de milestone: pending, in_progress, waiting_capacity, blocked, done. Uma tentativa pode ser partial, failed ou passed; isso não fecha o milestone automaticamente.

Marcar done somente com todos os critérios satisfeitos, links de evidência e aceite registrado. Teste não executado não passa. Planejado, implementado, revisado, instalado e publicado são fatos separados. Reabrir milestone apenas por mudança de contrato ou evidência invalidada, citando o motivo.

Para migração, instalação ou publicação: registrar intenção antes do efeito; depois registrar observado/confirmado. Se a sessão morrer entre os dois, usar outcome_unknown e reconciliar o estado real antes de repetir. Nunca repetir instalação ou migração apenas porque não há resposta salva.

## Retomada em qualquer cliente

1. Ler CHECKPOINT.md e somente as seções relevantes deste plano.
2. Conferir branch, HEAD, diff e existência das evidências apontadas.
3. Verificar se há escritor/processo anterior ativo; não disputar checkout nem remover locks por idade.
4. Reconciliar efeitos com resultado desconhecido antes de iniciar trabalho novo.
5. Ler contrato e evidências do milestone atual; não reabrir milestones concluídos sem drift.
6. Escolher modelo principal ou fallback disponível e registrar a escolha.
7. Executar a próxima tarefa, verificar, salvar recibo e atualizar checkpoint.

Contexto de despacho: objetivo, contrato, arquivos relevantes, evidências selecionadas, dúvidas e próxima ação. Alvo <=8 KiB; nunca enviar todo o histórico. Se insuficiente, dividir a tarefa ou usar referências explícitas, sem truncar requisitos.

## Persistência e Git

Arquivos salvos sobrevivem ao término da sessão e ao esgotamento de subscrição. Commits locais por milestone são o próximo nível de durabilidade; executar os checks exigidos antes de commits e incluir apenas arquivos da campanha. Push/publicação são etapas distintas e não devem ser presumidos. Até o commit, registrar explicitamente uncommitted no checkpoint.

O registro desta campanha não altera a política de retenção do laboratório existente em dev/fleet-smoke. Não modificar suas saídas transitórias por efeito colateral.

## Aceite final e restrições

Cumprir fmt, Clippy, testes, docs e guardrails conforme AGENTS.md. Preservar cliente <1ms, watchdog 3ms e zero disco/processos externos no hot path. Aplicar contratos atuais de context lookup e carga; não reviver gates históricos. Validar fail-open, concorrência, identidade, inicialização e recuperação. Não certificar ausência universal de perdas; declarar workload e resolver ou delimitar RES-004.

Dashboards não inferem produtividade de churn, sucesso de exit 0 ou encerramento de silêncio. Separar assinatura, rate limit e custo estimado. Ausência de sinal não é zero. Demonstrar exportação para receptor OTLP independente do SigNoz.

### Atualização M2 — 2026-10-06, hooks Codex

[Unidade concluída](evidence/M2/codex-hooks-01.md): descoberta efetiva trusted e contraste limitado de ambiente não restauraram entrega. Próximo diagnóstico deve observar o executor de hooks (shell/argv/status/pipe); parser PowerShell isolado não basta para corrigir o adaptador. AGY Flash medium prepara fixture, Grok Build Fast low revisa, Codex CLI gpt-5.6-luna medium somente se houver hipótese discriminante que exija inferência. M2 permanece in_progress.

### Atualização M2 — execução dos hooks Codex

[Diagnóstico](evidence/M2/codex-execution-01.md): eventos reais expõem falha exit1 dos três hooks; reprodução PowerShell confirma incompatibilidade do comando atual e entrega sintética com operador de chamada. Próxima entrega: candidato do adaptador com seleção de shell explícita, preservação de terceiros e revisão normal de confiança após mudança do hash. AGY Flash medium implementa, Grok Build Fast low revisa; sessão Luna medium somente depois da validação do candidato. Não fechar M2 nem declarar instalação corrigida antes da prova real.

### Atualização M2 — candidato e custo de shell

[Candidato01](evidence/M2/codex-candidate-01.md) implementado pelo coordenador após timeout AGY e revisado por Grok. Provas de shells passaram; custo de 1,5–2,6s por hook impede promoção nesta unidade. Próximo passo: reduzir esse custo e definir contrato de shell antes da revisão normal de trust e da sessão real. M2 permanece in_progress. Nenhuma instalação alterada.

### Atualização M2 — política explícita validada

[Candidato02](evidence/M2/codex-candidate-02.md): AGY contribuiu código, Grok orientou desenho; coordenador integrou. Mediana no pwsh caiu de 1199 para 204ms em nove amostras por modo. O modo direto exige PowerShell e remove o shell aninhado. Próxima unidade: projeção escopada, confiança legítima e sessão CLI real com Luna medium. M2 segue aberto; Desktop/IDE e SigNoz não certificados.

### Atualização M2 — projeção aplicada

[Aplicação escopada](evidence/M2/codex-activation-01.md) concluída com backup, preservação de terceiros e binários inalterados. Três hooks bridge agora modified no Codex; próxima ação externa é revisão humana em /hooks. Após confiança confirmada por consulta, executar uma sessão Luna medium com receptor privado. Não repetir consultas inferenciais de arquitetura ou implementação para esta aplicação já validada. M2 permanece aberto.
