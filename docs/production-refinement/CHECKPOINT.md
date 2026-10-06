# Checkpoint — refinamento de produção

Atualizado: 2026-10-06 (M2: três hooks Codex trusted e sessão real aprovada no receptor privado). Fonte do plano: [PLAN.md](PLAN.md).

## Posição atual

- Milestones: M0/M1 done; M2 in_progress (quatro CLIs entregaram ao receptor privado; lifecycle/semântica/E2E pendentes); M3 in_progress (pesquisa fonte concluída, runbook pendente); M4–M10 pending.
- Evidência M1: [inventory-01.md](evidence/M1/inventory-01.md), [aceite](evidence/M1/acceptance.md).
- Próximos fechamentos: M2 lifecycle Grok e persistência do daemon; M3 runbook da estação.
- Branch: docs/production-refinement-milestones.
- Base observada: a47a9ab (merge da release v0.6.0).
- Persistência: 723f7f0 é a base desta rodada; evidências Codex real02 e discussão de lifecycle preparadas para entrega. Reconciliar HEAD/remoto na retomada.
- Operação externa pendente: nenhuma aprovação de hooks. Samuel revisou e hooks/list confirmou os três trusted. Sessão real concluída e processos/pipes encerrados. Binários ativos preservados.
- Registro de encerramento da rodada anterior (2026-09-30; revalidar processos antes de executar): todos os executores encerrados. M1: Luna medium. M2: Sol high. M3: Flash medium via AGY não aceito ([recibo](evidence/M3/research-attempt-01.md)); fallback Sol medium concluiu pesquisa de código ([resultado](evidence/M3/source-research-02.md)). IDs são modelos solicitados; não inferir versão observada quando executor não a expõe.
- Resultado CLI real: [real-cli-01.md](evidence/M2/real-cli-01.md), [contrato](evidence/M2/agy-real-01-contract.md).
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

## Sessões reais (2026-10-06)

- AGY CLI Flash low: uma leitura real ->7 spans; sessão/modelo/step2 correspondem.
- Grok Build Fast low: uma leitura OAuth ->4 spans, incluindo2 Stop; nenhum span Claude na amostra, guarda importada ainda não provada isoladamente.
- Claude CLI: uma Read ->3 spans; login Pro/claude.ai. Alias haiku anunciou4.5, mas modelUsage registrou Sonnet5.5.
- Codex CLI0.154.0: gpt-6-luna rejeitado; gpt-5.6-luna medium leu marcador, porém zero frames de hook. Feature ativa/config/trust entries presentes não provam execução. Ambiente herdado CODEX_* pode influenciar; manter hipótese explícita.
- Provider Google indevido confirmado em sessões Grok/Claude. Tool-call IDs ausentes dos spans; correspondência desta amostra por sessão/ferramenta, não aceite de correlação geral.
- Todos os processos de teste encerrados, pipes removidos e hashes de hooks preservados. Receptor privado: SigNoz/Desktop/IDE não certificados.

## Nova unidade Codex (2026-10-06)

- App-server privado: três hooks bridge habilitados/trusted, sem erro de descoberta. Consulta sem turno de modelo.
- Nova leitura real Luna medium, retirando seis variáveis de identidade do coordenador: leitura correta, zero frames de hooks; daemon/harness encerrados, configuração preservada. Não equivale a terminal independente.
- Parser PowerShell rejeita comando sem `&`; shell efetivo do hook ainda desconhecido. Causa permanece aberta.
- Evidência e próximos testes: [codex-hooks-01](evidence/M2/codex-hooks-01.md). Nenhum teste ativo ou efeito pendente.

## Execução dos hooks delimitada (2026-10-06)

- App-server real com Luna medium: PreToolUse, PostToolUse e Stop falharam com exit1; a leitura e o turno completaram.
- Fonte pública 0.154.0 fixada: hook recebe shell do ambiente local, com fallback CMD.
- Prova sintética no PowerShell: comandos atuais exit1/ParserError; com operador & exit0 e três spans no receptor. Forte suporte ao defeito de compatibilidade, sem prova ainda de correção dentro do harness.
- [Evidências e aceites da correção](evidence/M2/codex-execution-01.md). Instalação e hooks globais preservados; processos e pipes de teste encerrados.

## Próxima tarefa concreta

1. [Codex real02](evidence/M2/codex-real-02.md) aprovado: três hooks completed e três spans correlacionados por sessão/trace no receptor privado. CLI atual0.160.1; não repetir o teste sem alteração ou falha nova. Contexto, tool-call IDs, Desktop/IDE e SigNoz continuam sem aceite.
2. [Revisão de lifecycle](LIFECYCLE-REVIEW.md) registrada a pedido de Samuel: SubagentStart/SubagentStop, SessionStart/SessionEnd e interrupções. Coleta M2, contrato M4, implementação M6 e consumo nos dashboards M5/M8. Não habilitar novos eventos antes do contrato e das provas.
3. Grok: localizar os dois Stop da mesma sessão e provar supressão do hook Claude importado. Capturar somente IDs/tipos/flags necessários, sem conteúdo de prompts.
4. M4/M6: corrigir semântica de provider desconhecido e mapear tool IDs após verificar payloads reais; sem correção semântica implementada ainda.
5. Investigar persistência normal do daemon e, depois dos contratos/correções, validar até SigNoz e superfícies Desktop/IDE. M3 runbook WSL continua independente.

Uma tarefa e um auxiliar por vez; até duas tentativas por unidade. Não fechar M2 com base somente na entrega de três CLIs ao sink privado.

## Revisão econômica de 2026-10-06

- AGY Flash low para coleta e manutenção; Flash medium para implementação delimitada e receitas verificadas.
- Grok Build Fast para diagnóstico e revisão comum; Grok 4.7 permanece consultor principal de arquitetura em pareceres curtos.
- Codex CLI gpt-5.6-luna medium validado como fallback econômico; Sol medium para criticidade concreta. High sem alocação automática.
- Inferências reais concluídas por AGY, Grok, Claude e Codex nesta unidade. Disponibilidade futura de subscrição permanece sujeita a preflight; catálogo não garante aceite de backend.
- Preservados aceites e estados dos milestones; pesquisa M3 já aceita não será refeita para trocar modelo.

## Preservar

O commit 68418c3 já inclui BUG_REPORT_AGY_WEBCHANNEL.md e mudanças de .gitignore. Preservar esse conteúdo e configurações de clientes. Esta unidade altera adaptador Codex, testes, contrato local, plano, checkpoint e suas evidências; não reverter mudanças anteriores nem incluir configurações ignoradas.

## Prompt de retomada

> Retome o refinamento de produção do agent-otel-bridge. Leia docs/production-refinement/CHECKPOINT.md e as seções necessárias de PLAN.md. Verifique branch, diff, escritor anterior e efeitos pendentes. Execute apenas a próxima tarefa pronta com o modelo indicado ou fallback registrado. Salve resultados antes de avançar. Use subscrições; não use APIs pagas. Pi está fora do escopo.

## Candidato Codex registrado

- [Implementação e limites](evidence/M2/codex-candidate-01.md): nove invocações nativas passaram em três shells, endpoint privado ausente. Sem sessão Codex real corrigida ou promoção.
- Prévia canônica disponível; hashes de trust devem ser obtidos pela revisão normal após definir a implementação final.
- Cargo guardrails passou (56,6s após build): fmt, Clippy, workspace tests, conformance, docs e cliente 152576 bytes. Processos auxiliares e testes encerrados; instalação/trust preservados.

## Candidato02 — política explícita

- AGY Flash medium entregou proposta de código; coordenador integrou e verificou. Grok Build Fast low orientou arquitetura. Duas consultas via subscrição, encerradas.
- Política preservada em reinstalação, erros explícitos, sem leitura de disco no cliente nativo. 36 invocações sintéticas passaram; instalação e trust inalterados.
- Cargo guardrails passou (64,0s após build): fmt, Clippy, workspace tests, conformance, docs e cliente 152576 bytes. Nenhum daemon ou harness inferencial iniciado nesta unidade.

## Aplicação Codex — aguardando revisão humana

- Base remota verificada: 9044f1e; esta unidade registra a aplicação e sua pendência. Reconciliar HEAD/remoto na retomada.
- Três comandos globais e política explícita aplicados; configuração de confiança e binários preservados. Recibos e backup em [codex-activation-01](evidence/M2/codex-activation-01.md).
- hooks/list exit0, quatro hooks encontrados, três bridge modified; nenhum processo de teste remanescente. Nenhum turno de modelo realizado nesta unidade.
- A pendência externa agora é a revisão de Samuel em /hooks. Declarações de instalação totalmente preservada acima descrevem rodadas anteriores. Não atestam o estado após esta aplicação escopada.

## Codex real02 — revisão concluída

- A pendência humana da rodada anterior foi resolvida. Três hashes trusted, três hooks completed e três spans; uma inferência Luna medium, sem API paga.
- Versão da estação mudou para0.160.1; comparação com a falha0.154.0 não isola a atualização. Durações reais839–947ms incluem shell, não SLA nativo.
- Próxima execução: diagnóstico delimitado dos dois Stop Grok e guarda Claude importada, com Grok Build Fast low/AGY Flash conforme plano. Não repetir inventário completo nem reabrir a confiança Codex sem mudança.
