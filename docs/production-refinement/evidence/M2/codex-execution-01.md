# M2 — falha de execução dos hooks Codex delimitada

2026-10-06. Base `cd14ca2`. Diagnóstico concluído nesta unidade; correção e M2 ainda abertos.

## O que está comprovado

| Prova | Resultado |
| --- | --- |
| Sessão real pelo app-server instalado, modelo gpt-5.6-luna, medium, read-only | Uma leitura, exit 0, marcador e resposta exatos; turno completed |
| Notificações de execução dos hooks globais do bridge | PreToolUse, PostToolUse e Stop: failed, cada um com `hook exited with code 1` |
| Bridge durante essa sessão | Zero eventos de hook; apenas dois frames de controle do supervisor |
| Reprodução sintética dos comandos configurados em PowerShell | Três exit 1 com ParserError/UnexpectedToken |
| Mesmos comandos e payloads sintéticos, adicionando apenas o operador `&` | Três exit 0, stdout `{}`, stderr vazio; três spans recebidos pelo sink privado |

A fronteira deixou de ser apenas “não chegou telemetria”: a sessão real tentou executar os hooks e eles falharam. Há defeito reproduzido de compatibilidade entre a string gerada pelo bridge e PowerShell. A cadeia de fonte e reprodução sustenta fortemente esse defeito como explicação da falha observada, mas a notificação real não contém o stderr nem o argv do processo. Não afirmar que o ParserError foi capturado dentro do Codex, nem que todas as causas foram eliminadas.

## Fonte fixada

Tag pública `rust-v0.154.0`, resolvida via GitHub para `6b9826e3aa83b1a5947db50f4332cb9c65f1b340`. A versão coincide com o CLI observado; não é uma atestação de build reproduzível do executável instalado. URLs e hashes: [sources.json](codex-execution-01/sources.json).

- [Construção da configuração](https://github.com/openai/codex/blob/6b9826e3aa83b1a5947db50f4332cb9c65f1b340/codex-rs/core/src/session/mod.rs#L4706): deriva programa/argumentos do shell do ambiente local quando disponível.
- [Inicialização da sessão](https://github.com/openai/codex/blob/6b9826e3aa83b1a5947db50f4332cb9c65f1b340/codex-rs/core/src/session/session.rs#L1339): fornece o ambiente local ao construtor de hooks.
- [Executor](https://github.com/openai/codex/blob/6b9826e3aa83b1a5947db50f4332cb9c65f1b340/codex-rs/hooks/src/engine/command_runner.rs#L390): usa o shell configurado, preserva snapshot de ambiente e tem fallback Windows para COMSPEC/cmd.exe quando não há shell configurado.

Logo, não tratar todo Codex Windows como consumidor fixo de CMD, nem aplicar um prefixo PowerShell indiscriminadamente a todo shell. O adaptador atual usa `install_standard_hooks`; o renderer existente `format_powershell_hook_command` já conhece o operador de chamada, mas não resolve sozinho a seleção de shell.

## Método e evidências

AGY Flash medium (modelo solicitado `gemini-3.8-flash-medium`, plan) orientou o cliente RPC limitado. Grok Build Fast low revisou a proposta sem ferramentas. Ambos responderam com exit 0; IDs efetivamente servidos não expostos no formato textual. Um auxiliar por vez, parent como único escritor. Mantida colaboração advisory da unidade anterior. Sem fallback API.

O coordenador incorporou as ressalvas do Grok: evento ausente não identifica causa; saída do hook não prova shell/argv; prazo e cleanup limitam o alcance. Por isso combinou evento real, fonte fixada e reprodução sintética. Rejeitou a generalização do AGY de que todo shell Windows seria PowerShell. A sessão real compara igualdade exata do marcador, não mera presença de substring.

Uma inferência real foi suficiente; nenhuma repetição de modelo. Cliente RPC: initialize/initialized, thread/start e turn/start, depois captura seletiva até turn/completed. Sem mudanças de configuração, trust ou sandbox. Shell comparison não usa modelo. Os dois supervisores usam o job Windows kill-on-close já validado, pipe exclusivo com PID conferido, receptor OTLP privado e deadline 150s; cliente RPC tem deadline 95s. Daemons e harnesses terminaram exit 0, app-server exit 0, hooks preservados, nenhum pipe residual. Consulta posterior não encontrou os processos de teste.

- [Sessão real e notificações](codex-execution-01/real-result.json), [recibo](codex-execution-01/real-receipt.json), [contadores](codex-execution-01/real-daemon.json).
- [Comparação dos seis comandos](codex-execution-01/synthetic-result.json), [recibo](codex-execution-01/synthetic-receipt.json), [três spans decodificados](codex-execution-01/synthetic-spans.json).
- [Cliente RPC](codex-execution-01/aob-codex-hook-execution.py), [supervisor](codex-execution-01/aob-codex-hook-supervisor.py), [fixture de shell](codex-execution-01/aob-codex-shell-repro.py), [supervisor sintético](codex-execution-01/aob-codex-shell-supervisor.py).
- [Hashes](codex-execution-01/sha256.json). Scripts são recibos reproduzíveis da estação, não ferramentas portáveis: copiar a fixture correspondente para `%TEMP%` com o nome original antes de executar o supervisor a partir do repo. Conferir hashes/binários e ausência de pipes; não executar ambos simultaneamente.

O campo histórico `harness_exe` do supervisor reutilizado identificava Codex embora o processo imediato fosse Python; os recibos selecionados deixam essa origem explícita e registram o harness real. Na prova sintética não houve modelo solicitado. Não interpretar esse metadado herdado como nova inferência.

## Decisão e próxima entrega

Preparar correção candidata do adaptador Codex Windows, com AGY Flash medium e revisão Grok Build Fast low. Aceites:

1. Gerar comando válido para o shell suportado, com política explícita para fallback CMD. Avaliar `commandWindows`/launcher explícito; não assumir que esse campo escolhe shell sozinho.
2. Preservar hooks terceiros e idempotência. Testar caminhos com espaços e caracteres especiais, captura de stdin, resposta JSON e fail-open. Medir overhead de shell separado do tempo nativo.
3. Não reutilizar trust antigo após mudar a definição. Preparar configuração candidata e hashes para revisão normal de confiança; nunca escrever uma aprovação automaticamente ou usar bypass para obter resultado verde.
4. Após projeção e confiança legítima, repetir uma sessão real: três hooks completed e eventos correlacionados até receptor privado. Só então tratar o caminho corrigido como validado; SigNoz e Desktop/IDE permanecem pendentes.

Nenhum código de produto, instalação ativa ou hook global foi alterado nesta unidade. O erro de provider Google em modelo ausente reapareceu na prova sintética e permanece encaminhado a M4/M6.

Validação: cargo guardrails passou em 46,7s; AST Python, links locais e hashes conferidos. Foram removidos 32 arquivos brutos de stream/payload pertencentes a estes dois testes após salvar as projeções; sessões internas dos harnesses preservadas.
