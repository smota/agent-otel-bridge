# Checkpoint — refinamento de produção

Atualizado: 2026-09-30. Fonte do plano: [PLAN.md](PLAN.md).

## Posição atual

- Milestones: M0/M1 done; M2 in_progress (tentativa 01 encerrada); M3 in_progress (pesquisa fonte concluída, runbook pendente); M4–M10 pending.
- Evidência M1: [inventory-01.md](evidence/M1/inventory-01.md), [aceite](evidence/M1/acceptance.md).
- Próximos fechamentos: M2 diagnóstico controlado; M3 runbook específico da estação.
- Branch: docs/production-refinement-milestones.
- Base observada: a47a9ab (merge da release v0.6.0).
- Persistência: arquivos locais uncommitted; nenhum commit ou push nesta rodada.
- Operação externa pendente: nenhuma; nenhum restart, upgrade ou instalação executado. Só leituras/API GET/SELECT, consultas a modelos e arquivos de documentação.
- Todos os executores desta rodada encerrados. M1: Luna medium. M2: Sol high. M3: Flash medium via AGY não aceito ([recibo](evidence/M3/research-attempt-01.md)); fallback Sol medium concluiu pesquisa de código ([resultado](evidence/M3/source-research-02.md)). IDs são modelos solicitados; não inferir versão observada quando executor não a expõe.
- Resultado M2: [diagnosis-01.md](evidence/M2/diagnosis-01.md).
- Upgrade verificado e lacunas do runbook: [upgrade-path-01.md](evidence/M3/upgrade-path-01.md).

## Decisões confirmadas com Samuel

- Quatro clientes: AGY, Claude, Codex e Grok. Pi excluído.
- SigNoz local no WSL; operação pessoal Windows como primeiro alvo.
- Todos os clientes usam subscrição; não usar fallback para API paga.
- Grok consultor principal; modelos escolhidos por tarefa, com fallbacks explícitos.
- Rodada enxuta; milestones pequenos e retomáveis, sem depender do histórico da conversa.

## Achados atuais, com evidências salvas

- Binários ativos 0.5.2 iguais a target/release, divergentes do manifesto/staging; origem do build desconhecida. Processo bridge e pipes ausentes na janela. Causa da ausência não comprovada.
- Collector Windows vivo e Codex nativo recente no ClickHouse; nenhuma linha bridge na janela24h. Histórico23/09 contém2448 spans bridge atribuídos AGY. Não confundir contagem de spans com tarefas.
- Hooks globais dos quatro clientes presentes; Claude/AGY codificados foram decodificados. Eficácia por sessão ainda não testada.
- SigNoz0.141.1 no WSL,18 dashboards publicados; compose /home/sam/pours/deployment/compose.yaml. Caminho obrigatório0.141.1 ->0.143.0 ->0.144.0; collector0.144.11 na parada0.143.
- Pesquisa de código fixada em SigNoz47dd1fab e collector a52dc570: mapping pode mover atributos, pricing não deduplica, totais dependem do escopo da query. Não implementar sem revisão M4.
- AgentFlow harness inspect: zero dos quatro pilares locais configurados; classificação council para risco alto, migração e contrato público. Nenhum run nativo criado.

Não repetir inventário inteiro na retomada: revalidar somente estado volátil e identidades relevantes à próxima ação.

## Próxima tarefa concreta

Sol high: definir e executar tentativa02 de M2 para capturar falha de inicialização com stdout/stderr e ambiente declarados, preferindo pipe/endpoint isolados. Registrar intenção, hash do binário, PID/identidade, término e limpeza antes do efeito; não sobrescrever instalação nem alterar hooks. Conferir variáveis suportadas pelo build observado antes de confiar no isolamento. Se isolamento não puder ser garantido, preservar evidência e completar o runbook M3 primeiro. M3 requer descobrir casting/gerador, pareamento de imagens, backup consistente, restauração e comandos específicos antes de fechar. Não repetir pesquisa de código já salva.

## Preservar

Checkout tinha entradas não rastreadas de configuração de clientes e BUG_REPORT_AGY_WEBCHANNEL.md antes desta documentação. Não adicionar, remover ou atribuir essas mudanças à campanha.

## Prompt de retomada

> Retome o refinamento de produção do agent-otel-bridge. Leia docs/production-refinement/CHECKPOINT.md e as seções necessárias de PLAN.md. Verifique branch, diff, escritor anterior e efeitos pendentes. Execute apenas a próxima tarefa pronta com o modelo indicado ou fallback registrado. Salve resultados antes de avançar. Use subscrições; não use APIs pagas. Pi está fora do escopo.
