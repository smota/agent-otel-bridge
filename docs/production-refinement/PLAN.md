# Refinamento de produção — milestones e continuidade

Data: 2026-09-30. Estado: planejamento salvo; execução técnica ainda não iniciada.

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
| M0 | Plano, checkpoint e regras de continuidade | Nenhuma | Coordenador atual | Arquivos salvos, coerentes e próximos passos explícitos |
| M1 | Linha de base: hashes reais, manifestos, processos, inicialização, hooks; Collector Windows; distro, serviços, versões e volumes WSL; dashboards publicados e geradores | M0 | gpt-6-luna medium / Codex | Inventário com caminhos, horários e diferenças; estado preservado antes de reiniciar ou instalar |
| M2 | Diagnóstico por cliente e por fronteira; atividade nativa versus bridge; duplicação, correlação, entrega e consultas | M1 | gpt-6.1-sol high / Codex; coleta delimitada com gemini-3.8-flash-medium / AGY | Quatro fichas de cobertura; hipóteses confirmadas/refutadas ou lacunas explícitas; causas suficientes para delimitar correções |
| M3 | Pesquisa upstream SigNoz e preparação do upgrade WSL: código, templates, convenções, migrações e backup/restauração | M1 | Pesquisa: gemini-3.8-flash-medium / AGY; migração: gpt-6.1-sol high / Codex | Relatório com commits/fontes, matriz adotar/adaptar/descartar e runbook com versão alvo fixada |
| M4 | Contratos de sinais e ADRs: independência OTLP, origem, deduplicação, correlação, dados ausentes, assinatura versus custo e rate limit | M2, M3 | grok-4.7 / Grok; síntese gpt-6.1-sol medium / Codex | Parecer Grok, revisão independente e objeções resolvidas; contratos aceitos pelo coordenador |
| M5 | Jornadas do operador; inventário de todos os painéis; manter/corrigir/mover/fundir/retirar; esboços e consultas propostas | M4 | claude-sonnet-4-6 / AGY | Mapa pergunta -> decisão -> sinal -> próximo passo e revisão com Samuel; principal com até seis painéis essenciais |
| M6 | Correções do bridge em candidato isolado, uma fatia por defeito; fixtures e regressões; contratos de runtime preservados | M4 | Adaptadores: gemini-3.8-flash-medium / AGY; instalação, IPC, watchdog e OTLP: gpt-6.1-sol high / Codex | Fatias revisadas; checks obrigatórios aprovados; evidência do candidato separada da instalação ativa |
| M7 | Ensaio de atualização SigNoz em cópia isolada e ensaio de restauração; executar atualização ativa após ensaio aprovado | M3, M4 | gpt-6.1-sol high / Codex | Identidade/versionamento, ingestão e consultas comparadas; recuperação exercitada; operação ativa reconciliada |
| M8 | Implementar dashboards e separação de assinaturas; validar consultas, dados ausentes e navegação | M5, M6, M7 | gemini-3.8-flash-medium / AGY | Evidência por painel, inspeção visual e jornada do operador validada |
| M9 | Instalação atômica do candidato; validação real dos quatro clientes; concorrência, reinício Windows/WSL e recuperação | M6, M7, M8 | gpt-6.1-sol high / Codex; revisão independente Sonnet | Matriz de aceite completa; hashes corretos; carga e limites declarados; receptor OTLP independente verificado |
| M10 | Fechar release: notas, limitações, runbook e índice final de evidências | M9 | gpt-6-luna medium / Codex; aceite gpt-6.1-sol medium | Pacote de lançamento pronto; publicação e implantação identificadas separadamente, sem inferir que ocorreram |

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

| Principal | Fallback autorizado para a mesma tarefa |
|---|---|
| gpt-6-luna medium | gemini-3.8-flash-low via AGY |
| gemini-3.8-flash-medium | gpt-6-luna medium para tarefas delimitadas; gpt-6.1-sol medium para pesquisa com síntese |
| gpt-6.1-sol medium | gemini-3.1-pro-high via AGY |
| gpt-6.1-sol high | claude-sonnet-4-6 via AGY; gemini-3.1-pro-high para IPC/concorrência |
| claude-sonnet-4-6 via AGY | gpt-6.1-sol medium para narrativa; high para revisão crítica |
| grok-4.7 | gpt-6.1-sol high produz parecer provisório; fechamento de M4 ainda requer Grok |

Modelo e cliente são campos separados. Sonnet via Claude direto pode substituir o transporte AGY após resolver e registrar o ID exato disponível; não tratar o alias sonnet como prova de uma versão específica.

O modelo revisor deve ser diferente do autor. Alterações de Sol são revisadas por Sonnet; alterações de Flash/Sonnet por Sol. Para mudança crítica, quem fez o diagnóstico não pode ser o único revisor. Registrar independência real; outra sessão no mesmo modelo não satisfaz essa regra.

gpt-6-astra high é escalada excepcional após duas tentativas com defeito concreto em unsafe/concorrência, latência ou semântica OTLP. Não usar para inventário, redação ou formatação. Falha de quota não justifica escalada de capacidade.

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
