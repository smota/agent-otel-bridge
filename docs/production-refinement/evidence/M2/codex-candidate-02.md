# M2 — candidato Codex 02: política explícita e custo menor

Data: 2026-10-06. Base: 423d8383bd841d855bed1d6c852d04f72c4cb345. M2 permanece in_progress. Unidade: candidato implementado e comparação de shells concluída; nenhuma projeção ativa ou aprovação de trust.

## Resultado e decisão

O instalador lê `agent-otel-bridge-policy.json` ao lado de hooks.json, fora do esquema do Codex. Esquema versão 1 com `windows_hook_shell` igual a `powershell` ou `portable`. O arquivo é lido somente durante instalação e permanece intacto. Configuração ausente mantém o candidato compatível anterior. JSON/campos/versões/modos inválidos e erros de leitura abortam sem substituir hooks. Não há inferência pelo shell do coordenador ou pelo comando antigo.

O modo powershell gera uma chamada direta ao executável canônico, com operador &, aspas duplas e escaping. Elimina o processo PowerShell adicional. Exige que o executor Codex use PowerShell; não cobre fallback CMD. O modo portable continua disponível para CMD/PowerShell. O contrato descreve essa escolha explicitamente; nenhum modo altera o shell escolhido pelo Codex. Nenhuma dependência ou alteração no cliente nativo, daemon, trust ou instrumentação.

Também corrigido o retorno do instalador: falhas agora produzem erro CLI, em vez de só imprimir [fail] e sair com 0. Instalações bem-sucedidas de outros clientes não são revertidas; o erro informa esse estado parcial. Um teste de subprocesso confirmou exit não zero e conteúdo preservado com política inválida.

## Comparação medida

Mesma máquina, binário e endpoint privado ausente; PATH vazio. Três eventos × três repetições × dois modos × dois shells = 36 invocações. Ordem dos modos alternada por repetição. Nenhum build/teste concorrente durante a coleta. Todas terminaram com exit 0, {}, stderr vazio. Tempos incluem criação do shell externo e hook; não medem o SLA nativo <1ms e não são p99.

| Shell externo | Portable, mediana | Direto, mediana | Redução |
| --- | ---: | ---: | ---: |
| Windows PowerShell | 2272,55ms | 960,93ms | 57,7% |
| pwsh distribuído com Codex | 1198,78ms | 203,65ms | 83,0% |

A cauda da amostra pwsh direto chegou a 542,6ms; não prometer latência fixa de 204ms. A diferença demonstra o benefício de remover o shell aninhado. O custo remanescente do executor é aceito para avançar à prova real deste candidato, sem aprovar produção ou redefinir SLAs.

[Recibo com 36 resultados e hashes](codex-candidate-02/comparison.json), [script](codex-candidate-02/compare.py), [política proposta](codex-candidate-02/agent-otel-bridge-policy.json), [prévia direta](codex-candidate-02/preview-powershell.json), [prévia portable](codex-candidate-02/preview-portable.json). Prévia bridge-only não substitui hooks globais.

[Projeção isolada da configuração atual](codex-candidate-02/projection.json): exatamente três strings command mudaram; terceiros e todos os demais valores JSON permaneceram iguais. Arquivo completo gerado em target, caminho e hashes no recibo. [Script para regenerar](codex-candidate-02/preview.py). Hashes SHA-256 não equivalem aos hashes de trust calculados pelo Codex. Hooks globais e config.toml mantiveram os hashes da unidade anterior. Nenhuma política gravada no home real.

## Colaboração e revisão

AgentFlow classificou bilateral/advisory/single-writer. Binding automático Claude baseado em version probe não substitui roteamento do usuário. Duas consultas, sequenciais, via subscrição; nenhum fallback API.

- Grok grok-4.7-build-fast, low, plan, no-subagents, max-turns1, sem web/ferramentas: consultoria arquitetural concluída, exit 0. Aceita recomendação de política explícita em metadados próprios. Rejeitadas inferência de política pelo texto antigo, caminho sem aspas e stub com configuração no hot path. Estimativas genéricas de latência do parecer não substituem medições. Não tratada a alegação de impossibilidade de um comando universal como prova formal.
- AGY gemini-3.8-flash-medium, plan, limite 60s, sem ferramentas: proposta de código entregue, exit 0. Coordenador adaptou Path para &str, retornos Result/String e testes para chamadas reais do instalador, sem dependência tempfile. AGY contribuiu implementação; coordenador foi o único escritor e fez a integração/verificação. IDs servidos não expostos no formato textual.
- Grok revisou o desenho, não o diff final. Revisão de código e síntese final pelo coordenador. Nenhuma sessão inferencial Codex nesta unidade.

Testes cobrem política estrita, preservação/idempotência, erro CLI e stdin Unicode/argumentos/resposta através dos modos portable e direto, com caminho especial e PATH vazio. Cargo guardrails passou em 64,0s após build: fmt, Clippy, workspace tests, conformance, docs e cliente 152576 bytes. native_context_process permanece ignored por exigir execução explícita; não apresentado como executado.

## Próxima unidade

Preparar a aplicação escopada das três alterações e da política powershell, usando os hashes/diff da projeção e backup. Revalidar que o Codex CLI alvo mantém PowerShell como shell de hooks; não generalizar para Desktop/IDE ou CMD. Mudança de definição exige revisão normal de confiança: não gravar trust automaticamente nem usar bypass.

Após confiança legítima, rodar uma única sessão gpt-5.6-luna medium supervisionada com receptor privado e exigir três hooks completed sem falha, evento de ferramenta correlacionado e encerramento. Se houver divergência de shell, corrigir a seleção antes da inferência. Runtime ativo 0.5.2 e divergência de manifesto continuam assuntos separados; projeção de hooks não prova upgrade do runtime. Depois: daemon persistente, Grok lifecycle, semântica e SigNoz/WSL. Não fechar M2 antecipadamente.