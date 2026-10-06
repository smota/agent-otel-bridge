# M2 — projeção Codex aplicada; revisão de confiança pendente

Data: 2026-10-06. Base: `9044f1e444b21863159cde7dd1ee665904576b46`.

## Resultado

Aplicada a projeção completa do candidato02 a `C:\Users\samue\.codex\hooks.json`, com somente três valores JSON alterados: comandos PreToolUse, PostToolUse e Stop do bridge receberam o operador PowerShell `&`. Terceiros e demais valores foram preservados. Criada a política explícita `agent-otel-bridge-policy.json` com versão1 e modo powershell.

Binário nativo ativo preservado; não houve promoção de binários, instalação completa, restart do daemon ou alteração de trust. O runtime antigo continua 0.5.2 e não conhece a política: não usar seu instalador para reprojetar os hooks. A atualização completa continua separada.

Consulta real `hooks/list`, sem turno de modelo: quatro hooks encontrados, três bridge habilitados, todos com `trustStatus=modified`, sem erros ou warnings. App-server encerrou exit0; configuração preservada pela consulta. Nenhuma inferência ou auxiliar foi acionado nesta unidade: aguardar confiança antes de consumir subscrição.

## Evidência e limites

- [Aplicação e backup](codex-activation-01/application.json), [script usado](codex-activation-01/apply.py), [descoberta após aplicação](codex-activation-01/hooks-list.json).
- Backup persistente: `C:\Users\samue\AppData\Local\agent-otel-bridge\backups\codex-hooks-86fe033e229949aab76bcde8bf5f7235\hooks.json`.
- SHA anterior: `0a9cc7b86c46c5bb32116fc973b85bc4e84587aea96d795c410e8c2622fad48c`.
- SHA aplicado: `f15b55003f78f6b67ff9c4efc253e4860183cc72a7b4ade64d7c378bd57f27b9`.
- SHA config.toml permaneceu `bd98b06f97c8569ba8408bde3a577a781b65c8b814915e0c386bed07236cb044`.
- Executável Codex manteve o SHA da investigação 0.154.0. A [fonte fixada do shell](https://github.com/openai/codex/blob/6b9826e3aa83b1a5947db50f4332cb9c65f1b340/codex-rs/shell-command/src/shell_detect.rs#L350) escolhe PowerShell no Windows; linhas267–280 priorizam pwsh. A resolução de PATH nesta rodada encontrou o pwsh do runtime Codex. Isso sustenta a escolha explícita para este ambiente CLI; não equivale a observar argv do hook real nem certifica Desktop/IDE ou fallback CMD.

## Ação humana necessária

No Codex CLI aberto neste repositório, usar `/hooks` e revisar os três comandos de `agent-hook.exe`, agora iniciados por `&`. Confirmar confiança somente para essas definições. A [documentação oficial](https://learn.chatgpt.com/docs/hooks#review-and-trust-hooks), consultada nesta rodada, informa que definições modificadas são puladas até nova confiança. O [contrato local, seção6](../../../local-runtime-contract.md#6-codex-windows-shell-policy) determina: “installers must not write trust approval”. Nenhuma aprovação foi escrita.

| Evento | Hash de definição retornado pelo Codex |
| --- | --- |
| PreToolUse | `sha256:fbedfbaa65849220515022c368346ee85fa03ff4e055738ac3aa6048ce822b90` |
| PostToolUse | `sha256:85f2c214006823959bd5fd4b4a641097d7248d0312e4ca496dcc812e1e29a56a` |
| Stop | `sha256:56aae8310c618ab1d852af10ff2ae7ef85618863f019dbc1ca9753096e62f3d2` |

Esses hashes de definição são distintos do SHA do arquivo. Retomar com consulta read-only e verificar confiança dos hashes atuais, sem reutilizar aprovação anterior.

## Próxima execução e recuperação

Verificação desta entrega: `cargo guardrails` passou (60,3s), incluindo fmt, Clippy, testes do workspace, conformance, documentação, cliente152576bytes e política de branch. O teste `native_context_process` permanece ignored conforme contrato existente; não foi executado. Essas verificações não substituem a sessão real pendente.

Após revisão: uma sessão supervisionada Codex `gpt-5.6-luna`, esforço medium, receptor OTLP privado, fixture existente. Exigir PreToolUse/PostToolUse/Stop concluídos sem falha e spans correlacionados. M2 permanece in_progress.

Se for necessário reverter esta projeção, primeiro confirmar que hooks e política ainda têm os hashes deste recibo. Restaurar atomicamente o backup completo e remover somente a política criada nesta rodada, após verificar seu hash. Preservar qualquer edição posterior; nunca restaurar por cima de drift. Não alterar confiança durante rollback. O script de aplicação é uma receita histórica de execução única, não um instalador idempotente.
