# M2 — candidato Codex Windows 01

Data: 2026-10-06. Base: e6bf1f02b09fa83a4446101984c37fdee18964fc. Estado: implementação candidata validada em shells; promoção pendente por latência e prova real. M2 continua in_progress.

## Resultado

O adaptador Codex agora projeta um launcher absoluto do Windows PowerShell com script UTF-16LE codificado. O script chama o caminho nativo entre aspas, com operador &, escaping e tag codex. O launcher funciona nos shells CMD, Windows PowerShell e pwsh testados, sem depender de PATH. Não certifica Bash/WSL como shell externo do Codex Windows.

SystemRoot deve ser absoluto e composto de caracteres seguros para um token sem aspas. Raiz ausente, com espaços ou metacaracteres é recusada antes da escrita. O caminho canônico do hook continua entre aspas dentro do script. Nenhuma dependência, cliente nativo ou daemon foi alterado.

A atualização preserva hooks terceiros, detecta comandos codificados, é idempotente e substitui overrides commandWindows pertencentes ao bridge. Overrides com propriedade ambígua e JSON inválido são recusados sem alterar o arquivo. Não há escrita de confiança/trust. A prévia não deve substituir o arquivo global completo: contém apenas os hooks bridge de um home isolado.

## Colaboração e custo

AgentFlow classify: bilateral; plan: advisory, single-writer. O binding automático Claude por version probe foi substituído pelo roteamento do usuário. Um auxiliar por vez, no máximo duas consultas nesta unidade.

- AGY: gemini-3.8-flash-medium, plan, print-timeout60s. Retornou aviso de timeout sem proposta, embora exit0. Tentativa não aceita; não declarar implementação AGY.
- Coordenador Codex: implementou o fallback no checkout após o timeout. Não foi lançada inferência CLI adicional para implementação.
- Grok: grok-4.7-build-fast, reasoning low, plan, no-subagents, max-turns1, sem web. Exit0; revisão textual do diff, sem ferramentas. Modelo efetivamente servido não exposto. Somente subscrições; sem API paga.

Grok pediu confirmação do decoder: ele já existe em is_bridge_command e os testes de idempotência/registro passaram. Foram incorporadas recusas para Windows-only/mixed ownership, validação de JSON antes de escrita e eliminação do fallback silencioso de evento desconhecido. Revisão cobriu o diff anterior a esses ajustes; alterações posteriores conferidas pelo coordenador e pelos guardrails. A hipótese de política PowerShell bloquear execução não foi testada nem tomada como fato universal.

## Evidências

- Teste Rust de processo: fixture nativa compilada recebe stdin JSON com Unicode e quebras de linha, verifica argumentos e retorna somente {}. Caminho contém espaço, $, &, backtick, % e !. Seis combinações (três eventos × CMD/PowerShell), PATH vazio, passaram. Tempo observado 1435–2186ms incluindo shells e fixture; não é benchmark p99.
- [Nove invocações nativas](codex-candidate-01/shell-results.json): candidato existente em target/release, hash registrado; pipe exclusivo ausente, três eventos × CMD/Windows PowerShell/pwsh. Exit0, {}, stderr vazio. 1514–2572ms; prova de fail-open desconectado, não entrega OTLP nem limite nativo <1ms.
- [Prévia gerada pelo CLI candidato](codex-candidate-01/hooks-preview.json), apontando ao caminho canônico. [Script da prova](codex-candidate-01/shell-probe.py). SHA-256 dos artefatos em [manifesto](codex-candidate-01/sha256.json); não são hashes de trust do Codex.
- Instalação ativa preservada. Hooks globais SHA256 0a9cc7b86c46c5bb32116fc973b85bc4e84587aea96d795c410e8c2622fad48c; config.toml bd98b06f97c8569ba8408bde3a577a781b65c8b814915e0c386bed07236cb044.

Durante desenvolvimento, o teste revelou que caminhos de executável CMD com barras / não funcionavam nessa invocação; normalizados para barras Windows. A primeira execução nativa usou nome de variável de pipe incorreto e não foi aceita como prova de isolamento; repetida com AGENT_OTEL_PIPE e endpoint completo exclusivo. O recibo contém somente a execução corrigida. Nenhum daemon foi iniciado.

## Aceite e próxima unidade

Compatibilidade de shells, argumentos/stdin, resposta JSON, fail-open desconectado e preservação na instalação: passaram no escopo descrito. Cargo guardrails passou em 56,6s após build: fmt, Clippy, workspace tests, conformance, docs e tamanho do cliente 152576 bytes. Teste native_context_process permanece ignored por contrato de execução explícita; esta unidade não o apresenta como executado.

**Não promover ainda.** O launcher resolve a incompatibilidade de sintaxe, mas adiciona segundos à chamada síncrona. Próxima unidade: estudar comando direto com seleção explícita de shell ou launcher de menor custo; comparar com o baseline sem modelo e definir contrato de suporte/overhead. Grok Build Fast low para revisão arquitetural curta; AGY Flash medium para alteração delimitada se disponível. Uma tentativa por auxiliar e fallback registrado.

Após resolver esse custo: preparar configuração final, passar pela revisão normal de confiança do Codex e executar uma sessão real Luna medium. Exigir três hooks completed e spans correlacionados. Não alterar trust automaticamente. SigNoz, Desktop/IDE, persistência do daemon e demais defeitos semânticos permanecem pendentes.