# Checkpoint — refinamento de produção

Atualizado: 2026-10-06 (revisão do plano e verificação Git; runtime não revalidado). Fonte do plano: [PLAN.md](PLAN.md).

## Posição atual

- Milestones: M0/M1 done; M2 in_progress (tentativa 01 encerrada); M3 in_progress (pesquisa fonte concluída, runbook pendente); M4–M10 pending.
- Evidência M1: [inventory-01.md](evidence/M1/inventory-01.md), [aceite](evidence/M1/acceptance.md).
- Próximos fechamentos: M2 diagnóstico controlado; M3 runbook específico da estação.
- Branch: docs/production-refinement-milestones.
- Base observada: a47a9ab (merge da release v0.6.0).
- Persistência: antes da entrega de 2026-10-06, HEAD 68418c3b5f0aeb5a2797265c15741512176949b6 continha plano e evidências anteriores; a branch ainda não existia em origin e não tinha upstream. Samuel autorizou commit e push da revisão de PLAN.md/CHECKPOINT.md para origin/docs/production-refinement-milestones. Este registro acompanha o commit de entrega; na retomada, comparar git rev-parse HEAD com git ls-remote --heads origin refs/heads/docs/production-refinement-milestones para confirmar o estado remoto, sem inferir sucesso apenas da intenção.
- Operação externa pendente: nenhuma; nenhum restart, upgrade ou instalação executado. Só leituras/API GET/SELECT, consultas a modelos e arquivos de documentação.
- Registro de encerramento da rodada anterior (2026-09-30; revalidar processos antes de executar): todos os executores encerrados. M1: Luna medium. M2: Sol high. M3: Flash medium via AGY não aceito ([recibo](evidence/M3/research-attempt-01.md)); fallback Sol medium concluiu pesquisa de código ([resultado](evidence/M3/source-research-02.md)). IDs são modelos solicitados; não inferir versão observada quando executor não a expõe.
- Resultado M2: [diagnosis-01.md](evidence/M2/diagnosis-01.md).
- Upgrade verificado e lacunas do runbook: [upgrade-path-01.md](evidence/M3/upgrade-path-01.md).

## Decisões confirmadas com Samuel

- Quatro clientes: AGY, Claude, Codex e Grok. Pi excluído.
- SigNoz local no WSL; operação pessoal Windows como primeiro alvo.
- Todos os clientes usam subscrição; não usar fallback para API paga.
- Grok consultor principal; modelos escolhidos por tarefa, com fallbacks explícitos.
- Rodada enxuta; milestones pequenos e retomáveis, sem depender do histórico da conversa.

## Achados de 2026-09-30, com evidências salvas

- Binários ativos 0.5.2 iguais a target/release, divergentes do manifesto/staging; origem do build desconhecida. Processo bridge e pipes ausentes na janela. Causa da ausência não comprovada.
- Collector Windows vivo e Codex nativo recente no ClickHouse; nenhuma linha bridge na janela24h. Histórico23/09 contém2448 spans bridge atribuídos AGY. Não confundir contagem de spans com tarefas.
- Hooks globais dos quatro clientes presentes; Claude/AGY codificados foram decodificados. Eficácia por sessão ainda não testada.
- SigNoz0.141.1 no WSL,18 dashboards publicados; compose /home/sam/pours/deployment/compose.yaml. Rota pesquisada naquela data: 0.141.1 ->0.143.0 ->0.144.0; collector0.144.11 na parada0.143. Revalidar release alvo e migrações antes do ensaio; 0.144.0 não está certificado como latest em 2026-10-06.
- Pesquisa de código fixada em SigNoz47dd1fab e collector a52dc570: mapping pode mover atributos, pricing não deduplica, totais dependem do escopo da query. Não implementar sem revisão M4.
- AgentFlow harness inspect: zero dos quatro pilares locais configurados; classificação council para risco alto, migração e contrato público. Nenhum run nativo criado.

Não repetir inventário inteiro na retomada: revalidar somente estado volátil e identidades relevantes à próxima ação.

## Próxima tarefa concreta

1. AGY / gemini-3.8-flash-low: preparar M2 tentativa02. Revalidar somente hash/build, processos/pipes e variáveis de isolamento suportadas. Salvar comandos propostos, resultado esperado, stdout/stderr previstos e limpeza. Ler docs/local-runtime-contract.md. Nenhum restart, alteração de hook ou instalação nesta preparação.
2. Grok / grok-4.7-build-fast: revisar em leitura o contrato de isolamento e as hipóteses. Havendo risco não resolvido, obter parecer delimitado grok-4.7; revisão crítica conforme PLAN.md. Não presumir que variável desconhecida isola o binário.
3. AGY / gemini-3.8-flash-medium: executar somente a receita isolada verificada, registrando intenção, hash, PID/identidade, término, stdout/stderr e limpeza. Se isolamento não for demonstrável, salvar lacuna e avançar M3; não usar a instalação ativa como fallback.
4. Alternativa independente M3: AGY / gemini-3.8-flash-medium completa runbook da estação (gerador/customizações do Compose, pareamento de imagens, backup consistente, restauração e comandos). Grok Build Fast confere fontes; Grok 4.7 revisa riscos da migração. Reutilizar a pesquisa aceita, revalidando versões voláteis.

Antes de cada despacho, salvar contrato e modelo; uma tarefa e um auxiliar por vez, até duas tentativas. Falha de subscrição usa fallback da matriz ou waiting_capacity. Não iniciar implementação com base apenas nesta revisão documental.

## Revisão econômica de 2026-10-06

- AGY Flash low para coleta e manutenção; Flash medium para implementação delimitada e receitas verificadas.
- Grok Build Fast para diagnóstico e revisão comum; Grok 4.7 permanece consultor principal de arquitetura em pareceres curtos.
- Codex Luna como fallback econômico; Sol medium para criticidade concreta. High sem alocação automática.
- grok models confirmou login grok.com na repetição; agy models retornou o catálogo. Nenhuma nova inferência nesses clientes foi executada nesta revisão; disponibilidade de quota continua não comprovada.
- Preservados aceites e estados dos milestones; pesquisa M3 já aceita não será refeita para trocar modelo.

## Preservar

O commit 68418c3 já inclui BUG_REPORT_AGY_WEBCHANNEL.md e mudanças de .gitignore. Preservar esse conteúdo e configurações de clientes. Esta revisão altera somente PLAN.md e CHECKPOINT.md; não reverter mudanças anteriores nem incluir configurações ignoradas.

## Prompt de retomada

> Retome o refinamento de produção do agent-otel-bridge. Leia docs/production-refinement/CHECKPOINT.md e as seções necessárias de PLAN.md. Verifique branch, diff, escritor anterior e efeitos pendentes. Execute apenas a próxima tarefa pronta com o modelo indicado ou fallback registrado. Salve resultados antes de avançar. Use subscrições; não use APIs pagas. Pi está fora do escopo.
