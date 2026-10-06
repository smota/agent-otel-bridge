# Checkpoint — refinamento de produção

Atualizado: 2026-10-06 (M2: duas sondagens isoladas concluídas; estação ativa preservada). Fonte do plano: [PLAN.md](PLAN.md).

## Posição atual

- Milestones: M0/M1 done; M2 in_progress (tentativa 02 encerrada; integração real pendente); M3 in_progress (pesquisa fonte concluída, runbook pendente); M4–M10 pending.
- Evidência M1: [inventory-01.md](evidence/M1/inventory-01.md), [aceite](evidence/M1/acceptance.md).
- Próximos fechamentos: M2 diagnóstico controlado; M3 runbook específico da estação.
- Branch: docs/production-refinement-milestones.
- Base observada: a47a9ab (merge da release v0.6.0).
- Persistência: plano-base 67ef649 confirmado no remoto na entrega anterior. Evidências e checkpoint da tentativa02 preparados para commit/push nesta rodada; reconciliar HEAD com origin/docs/production-refinement-milestones na retomada. A intenção de entrega não substitui verificação remota.
- Operação externa pendente: nenhuma. Dois processos de teste em ambiente privado foram encerrados e reconciliados; nenhum daemon permanente, restart global, upgrade ou instalação. Hooks preservados.
- Registro de encerramento da rodada anterior (2026-09-30; revalidar processos antes de executar): todos os executores encerrados. M1: Luna medium. M2: Sol high. M3: Flash medium via AGY não aceito ([recibo](evidence/M3/research-attempt-01.md)); fallback Sol medium concluiu pesquisa de código ([resultado](evidence/M3/source-research-02.md)). IDs são modelos solicitados; não inferir versão observada quando executor não a expõe.
- Resultado M2: [diagnosis-01.md](evidence/M2/diagnosis-01.md), [tentativa02](evidence/M2/diagnosis-02.md), [contrato](evidence/M2/diagnosis-02-contract.md).
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

## Resultado novo de M2 (2026-10-06)

- Sem daemon/pipes na estação; Collector ativo e backend com Codex nativo na última hora. Hashes ativos continuam divergentes do manifesto.
- Mesmo binário ativo funcionou em foreground e via start isolado. Hook nativo enviou quatro eventos sintéticos; quatro spans decodificados em cada teste, um por cliente. Job/pipe limpos; não comprova integração real dos harnesses.
- Defeito reproduzido: modelo ausente resulta em provider Google para todos os clientes. Agente correto, provedor fictício; encaminhado a M4/M6.
- Causa da ausência do daemon normal permanece desconhecida. Não inferir falha de startup, desativação pelo Windows ou correção a partir desses testes.

## Próxima tarefa concreta

1. AGY / gemini-3.8-flash-low: desenhar teste de uma sessão AGY real com daemon privado supervisionado, herança de pipe e evento marcado inofensivo. Preservar hooks e autenticação; não copiar credenciais para evidência. Separar superfície CLI de IDE/Desktop.
2. Grok / grok-4.7-build-fast revisa a receita; AGY / gemini-3.8-flash-medium executa somente com isolamento verificável. Salvar contrato, resultado e cleanup antes de avançar aos demais clientes.
3. Repetir por Grok (guarda Claude/importação), Claude e Codex com modelos econômicos disponíveis. Medir eventos reais e atribuição; não usar exit0 do hook como prova de entrega.
4. Investigar lifecycle no ambiente normal como unidade separada: stdout/stderr e ambiente efetivo no login, sem remover isolamento ou reinstalar por tentativa. O teste com home vazio não cobre providers/configurações reais.
5. M3 independente: completar runbook WSL com Flash medium; Grok revisa fontes e riscos. Reutilizar pesquisa aceita e revalidar versão alvo antes do ensaio.

Uma tarefa e um auxiliar por vez; no máximo duas tentativas por unidade. Não fechar M2 antes da matriz real. Nenhuma correção de produto foi implementada nesta rodada.

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
