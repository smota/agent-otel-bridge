# M2 — descoberta efetiva dos hooks Codex

Data: 2026-10-06. Base: `69acfe4`. Unidade diagnóstica concluída; M2 continua aberto.

## Resultado

- O app-server privado do Codex 0.154.0 descobriu quatro hooks: três do bridge e um de terceiro. Os três do bridge estavam `enabled=true`, `trustStatus=trusted`, com hashes efetivos distintos e sem warnings/errors. Isso vale para esta consulta, cwd e ambiente; não comprova execução na sessão anterior.
- Uma nova sessão CLI `gpt-5.6-luna`, medium, read-only, executou exatamente uma leitura e devolveu o marcador correto. Remover seis variáveis de identidade do coordenador não restaurou a entrega: zero eventos admitidos/transformados; só HealthPing e Shutdown do supervisor chegaram ao daemon.
- O parser PowerShell rejeitou a string atual de PreToolUse com `UnexpectedToken`; adicionar `&` eliminou o erro de sintaxe. Este teste não executou o hook. O shell efetivo do executor de hooks ainda não foi identificado: quoting é hipótese, não causa comprovada.
- Consulta read-only de logs locais, limitada à nova thread e a 20 linhas com referência a hooks, retornou zero linhas. Ausência de logs não prova ausência de tentativa.

## Contrato executado e limites

Primeiro, consulta sem inferência: `initialize`, notificação `initialized`, `hooks/list` sobre stdio de processo privado. Nenhum thread/turn iniciado, nenhuma decisão de confiança alterada. Deadline RPC de 30s, stdin fechado e espera de 5s; o processo saiu normalmente com código 0. Consulta posterior não encontrou filhos diretos daquele PID. Não foi um teste de hooks em execução.

Depois, uma inferência no supervisor já verificado em [real-cli-01](real-cli-01.md), com job Windows kill-on-close, deadline, pipe exclusivo, PID do servidor confirmado e receptor HTTP local. Mesmos binários ativos, hashes fixados, hooks globais preservados. Variáveis retiradas somente no processo filho:

`CODEX_THREAD_ID`, `CODEX_SESSION_ID`, `CODEX_CI`, `CODEX_INTERNAL_ORIGINATOR_OVERRIDE`, `CODEX_APP_TOOLS_PIPE_PATH`, `CODEX_TASK_WORKSPACE_VERIFYING_IDENTITY`.

Home, dependências e variáveis de sandbox foram preservados; chaves de API removidas como no supervisor anterior. Este é um contraste limitado do ambiente herdado, não uma execução a partir de terminal independente. Modelo e sandbox confirmados no rollout da thread `01a10ed2-c48a-7633-a64d-97eb8cf2b046`. Daemon/harness saíram 0; nenhum pipe residual. O aviso de flush do rollout também persistiu; causalidade desconhecida.

## Colaboração econômica

AgentFlow classificou esta unidade como bilateral/advisory, single-writer. O plano automático sugeriu Claude a partir de probe de versão; a preferência explícita do usuário foi aplicada: AGY e Grok, sequenciais, somente aconselhamento. Nenhum subagente interno criado.

- AGY, solicitado `gemini-3.8-flash-low`, plan: propôs descoberta por stdio. Resposta recebida, exit 0. ID efetivamente servido não exposto no formato textual.
- Grok, solicitado `grok-4.7-build-fast`, low, plan: confirmou a utilidade da consulta e destacou que trust não prova execução, nem app-server privado reproduz exec. Resposta recebida, exit 0. A primeira chamada foi rejeitada localmente por formato `text`; corrigido para `plain`, sem inferência na chamada rejeitada.
- Síntese do coordenador: descartados `shutdown` RPC não verificado, estado `pending` ausente do schema e conclusão do AGY de que trust isolaria a falha exclusivamente em process-spawn. Usados fechamento de stdin e enum do schema local. A ressalva do Grok sobre filhos foi acompanhada de consulta de processos; a inferência usou job supervisionado.

Limite da unidade: uma consulta sem modelo, uma inferência Codex, uma resposta por consultor. Sem fallback de API, mudança de trust, instalação, upgrade ou edição de hooks.

## Evidências e reprodução

- [Consulta efetiva](codex-hooks-01/hooks-list.json), [script de consulta](codex-hooks-probe.py).
- Schemas gerados pelo próprio CLI instalado: [parâmetros](codex-hooks-01/HooksListParams.json), [resposta](codex-hooks-01/HooksListResponse.json).
- [Supervisor do contraste](codex-hooks-01/env-probe.py), [recibo](codex-hooks-01/env-receipt.json), [sessão selecionada](codex-hooks-01/env-session.json), [contadores do daemon](codex-hooks-01/daemon-stderr.json).
- [Hashes dos artefatos](codex-hooks-01/sha256.json). Projeções sem prompts/transcrições completos; stream bruto e payloads do receptor desta unidade removidos após extração/hash. Sessão interna do harness preservada.

A [documentação oficial de hooks](https://learn.chatgpt.com/docs/hooks) consultada nesta data explica confiança vinculada ao hash atual e descoberta por camadas. A decisão diagnóstica usa principalmente o schema e a resposta do executável instalado; não presume equivalência entre documentação corrente e versão instalada.

## Próxima unidade

1. Identificar o executor de comandos de hooks do Codex 0.154.0 e capturar resultado de execução sem alterar trust: shell/argv, tentativa de processo, status/timeout e presença das variáveis do pipe. Preferir fonte/diagnóstico local, sem nova inferência até definir observação discriminante. Grok Build Fast low revisa; AGY Flash medium prepara fixture limitada.
2. Se PowerShell for confirmado, preparar correção candidata do adaptador com prova de chamada e compatibilidade; não aplicar `&` globalmente com base apenas no teste de parser.
3. Continuam pendentes: duas ocorrências Stop do Grok, supressão importada Claude, provider desconhecido, tool IDs, daemon persistente e E2E até SigNoz/Desktop/IDE.

Trust ausente e aquelas seis variáveis não explicam isoladamente os resultados desta unidade. A causa da falta de frames continua aberta.
