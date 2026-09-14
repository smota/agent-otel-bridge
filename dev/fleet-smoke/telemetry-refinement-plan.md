# Plano de refinamento do relatório e da telemetria

Estado: implementado no laboratório de desenvolvimento; validação final registrada abaixo. Este documento complementa o laboratório existente. A implementação não ativa campanhas, provedores, instalação ou consultas a backends reais.

Implementação: R1 em `report.rs` e no schema JSON; R2 em `telemetry.rs` e `telemetry_validation.rs`; R3/R4 em `runner.rs`; R5 em `main.rs` e `visibility.rs`; R6 em `report.rs`/CLI; R7 nos verificadores e testes; R8 na divisão Luna/Terra usada nesta entrega. O backend real continua dependente do canal escolhido, como previsto no escopo de R5.

Decisões de implementação: o teste de progresso injeta um intervalo curto em vez de simular o relógio completo; a produção usa 15 segundos. Ao atingir 240 spans de progresso, a emissão para e o resumo indica o limite, sem interromper os casos. Erros de transporte interrompem novas ondas, mas o fechamento terminal ainda faz sua própria tentativa única. Um erro nessa tentativa pode atualizar o relatório local depois de o root já ter sido enviado; não há reenvio do mesmo span. A revisão de fonte é opcional e fica desconhecida quando não fornecida. O caminho legado `Runner::run` retém artefatos por compatibilidade; CLI e `run_outcome` são transitórios por padrão.

## Objetivo e critérios de conclusão

Localizar uma execução pelo trace ID, reconstruir os casos e suas expectativas somente pela telemetria recebida e comparar o resultado com a especificação e a implementação. O relatório local deve continuar útil quando a exportação falhar. Código e contratos ficam versionados em `dev/fleet-smoke`; dados de execução continuam transitórios.

A conclusão exige todos os critérios R1–R8 abaixo, com testes locais e evidências separadas por superfície. Aceitação HTTP não equivale a visibilidade no backend; visibilidade de spans do laboratório não comprova propagação nativa do bridge.

## Lacunas confirmadas na implementação atual

- IDs existem em cada span, mas faltam um resumo de correlação e um resultado terminal estruturado em falhas do runner/exportador.
- O CLI acumula recibos entre repetições; cada execução precisa de seus próprios lotes e contadores.
- A exportação progressiva inclui seed/profile, mas não todos os campos disponíveis no plano/relatório. `export_run` e o caminho progressivo têm metadados diferentes.
- Faltam links de navegação opcionais, janela de busca e verificação explícita de visibilidade.
- O root só é exportado quando termina; acompanhar uma execução longa depende de evidência de progresso já encerrada e exportável.

## Contrato de saída v2 — R1

Preservar a saída JSON padrão em array e os campos existentes `run`, `verification` e `export`. Acrescentar `schema_version: 2` e `summary` a cada item; não remover os IDs por span. Implementar um leitor que aceite artefatos v1 e v2 e rejeite versões desconhecidas com erro estruturado.

| Grupo | Campos obrigatórios / semântica |
|---|---|
| Identidade | `run_id`, `trace_id`, `root_span_id`, índice da repetição e total solicitado; IDs reservados antes de operações sujeitas a falha |
| Reprodução | seed, perfil, versão do cenário, hash SHA-256 do plano canônico, versão do laboratório e revisão de fonte opcional |
| Estado | `completed`, `failed`, `cancelled` ou `incomplete`; motivo categorizado; contagens de casos planejados, iniciados, encerrados e não executados |
| Tempo | início/fim UTC RFC3339 e Unix nanos; duração monotônica; fim nulo enquanto não concluído; janela de busca com margem explícita de 30 segundos |
| Avaliação | resultados separados para casos, estrutura do trace, propagação nativa, transporte e visibilidade no backend |
| Navegação | serviço, trace ID copiável, links opcionais, estado da retenção do artefato e caminho apenas quando existir |

O plano deve ser canônico por ordenação estável e algoritmo versionado. Não executar `git` na trajetória de telemetria: revisão é metadado opcional fornecido ao laboratório antes da execução; valor ausente permanece desconhecido. Seed reproduz estímulos, não respostas de LLM, timestamps ou escalonamento do sistema operacional.

## Paridade entre relatório e OTLP — R2

Criar um único `RunContext` e um único mapeador para exportação progressiva e completa. Todos os lotes carregam identidade e reprodução; cada span de caso carrega o contexto necessário para avaliação independente.

| Escopo OTLP | Conteúdo proposto |
|---|---|
| Resource | `service.name`, versão do laboratório, `agent.smoke.run_id`, seed, perfil, versão do cenário, hash do plano, versão do contrato, modo e revisão opcional |
| Span | trace/span/parent IDs e links OTLP nativos; task ID, operação, plataforma planejada, modelo/reasoning configurados, origem e fase |
| Expectativa | `agent.smoke.expected_fault` e resultado esperado; referências à especificação e à função implementada |
| Observação | falha observada, resultado da avaliação, categoria de erro e evidência resumida com origem |
| Modelo observado | valor e fonte somente quando comprovados pelo provedor; ausência não substituída pelo modelo configurado |

Documentar os atributos `agent.smoke.*` no dicionário do laboratório; não alterar convenções do core nem inventar evidência nativa. Em OTLP, ausência é atributo omitido; para casos sem falha esperada, usar valor explícito `none` para distingui-los de ausência de evidência. O status técnico ERROR de uma falha inserida pode coexistir com avaliação PASS do caso. Nenhum prompt, credencial, stdout bruto ou URL com segredo entra nos atributos. Limitar valores textuais a 2 KiB e registrar truncamento.

## Falhas parciais e finalização — R3

Reservar identidade, registrar casos planejados e iniciar um acumulador em memória antes da execução. Toda saída recuperável do runner, timeout, cancelamento suportado, erro de exportação ou limpeza passa por um finalizador único. Ele encerra spans locais abertos, marca casos não executados e produz relatório terminal com a evidência já obtida. Não emitir spans de execução fictícios para tarefas que nunca começaram.

Falha de transporte continua encerrando a execução com código 2: concluir a onda já iniciada, recolher seus resultados, impedir novas ondas e tentar a finalização local. Não adicionar retries automáticos de exportação nesta etapa; registrar resultado de rede incerto como `unknown`, pois o servidor pode ter recebido o lote. Códigos: 0 para verificações exigidas aprovadas; 1 para falha/inconclusão de avaliação exigida; 2 para falha operacional. Erro de limpeza não apaga a causa original.

O contrato cobre falhas tratáveis e cancelamento cooperativo. Kill forçado, crash do processo ou stdout fechado podem impedir o relatório final; sem persistência obrigatória, não prometer recuperação posterior.

## Progresso e trace longo — R4

Adicionar saída opcional JSONL com registros `run_started`, `wave_completed`, `export_receipt` e `run_finished`, todos com run/trace IDs e sequência crescente. Manter JSON em array como padrão. Não escrever dados de execução em arquivo por padrão.

Exportar spans filhos assim que a onda termina e um pequeno span de progresso a cada 15 segundos enquanto houver trabalho ativo. Todos compartilham o trace e referenciam o root reservado. O root é exportado uma única vez ao encerrar; não reenviar versões parciais com o mesmo span ID. O backend pode mostrar filhos antes do root: isso é esperado e deve ser documentado.

Limitar progresso a 240 spans por execução, informar quando atingir o limite e separar contagens de spans de controle das de casos. Usar relógio injetável nos testes para não esperar minutos. Duração/pacing de campanhas continua um trabalho separado; este refinamento torna observável uma tarefa longa sem fabricar duração.

## Recibos e visibilidade — R5

Cada lote terá `batch_id`, run/trace IDs, sequência, span IDs, contagem enviada, instante e duração da tentativa, status HTTP opcional, rejeições reportadas e resultado `accepted`, `rejected` ou `unknown`. Resetar a coleção a cada repetição. `partialSuccess` não identifica necessariamente quais spans foram rejeitados: não inventar essa associação.

Visibilidade terá estados `not_checked`, `visible_partial`, `visible_complete`, `not_found` e `query_failed`, com instante, janela consultada, IDs esperados/observados/faltantes e fonte. Importar arquivo OTLP comprova apenas asserções daquela captura. Somente uma consulta efetiva ao backend configurado pode gerar resultado de visibilidade. Nenhum desses estados, isoladamente, comprova propagação nativa.

Nesta entrega, implementar o contrato e a interface de consulta com backend falso local; sem backend real configurado, manter `not_checked`. Um adaptador real depende da escolha do canal/API e de acesso já autorizado, sem bloquear os demais itens do plano.

## Navegação e retenção — R6

Aceitar template opcional de URL de leitura com placeholders permitidos `{trace_id}`, `{start_unix_ms}` e `{end_unix_ms}`. Validar esquema HTTP(S), rejeitar credenciais embutidas e placeholders desconhecidos, codificar valores e não abrir a URL automaticamente. Sem template, emitir IDs, serviço e janela de busca; não adivinhar sintaxe de SigNoz ou outro backend.

Por padrão, stdout e buffers em memória são a única retenção do relatório. `--keep-artifact` conserva explicitamente o artefato v2 temporário completo, inclusive em falha tratável. Reportar `retained`, `removed`, `not_created` ou `cleanup_failed`; não apresentar caminho apagado como arquivo disponível. Conservar a proteção de limpeza contra caminhos externos e arquivos inesperados.

## Matriz de validação e aprendizado — R7

| Critério | Evidência positiva | Contraprova obrigatória |
|---|---|---|
| R1 | Resumo e spans concordam; leitura v1/v2; mesma seed preserva hash | IDs divergentes, plano adulterado e versão desconhecida são rejeitados |
| R2 | Comparar atributos de cada lote com plano/relatório | Remover expectativa, trocar modelo planejado ou referência deve falhar; segredo sentinela nunca aparece |
| R3 | Falha na segunda onda preserva primeira onda e relatório terminal | Timeout, falha de spawn, exportação e limpeza não somem nem viram sucesso |
| R4 | IDs disponíveis no início; progresso antes do root; root único | Relógio falso, cancelamento e limite de progresso não criam duplicatas ou loops |
| R5 | Duas repetições têm recibos isolados; backend falso retorna cobertura | HTTP 200, aceitação parcial, perda de conexão e captura local não viram visibilidade completa |
| R6 | Link correto e retenção fiel ao filesystem | URL com credencial/template inválido e limpeza fora do diretório permitido são recusados |
| R8 | Ferramenta ausente e modelo não comprovado permanecem explícitos | Nunca substituir modelo silenciosamente nem tratar laboratório como bridge nativo |

Cada asserção retorna `requirement_id`, referência de implementação, esperado, observado, resultado e origem da evidência. Divergências são classificadas como defeito da implementação, especificação ambígua, limitação do ambiente ou evidência insuficiente. Emitir resumo de divergências no relatório e na telemetria terminal quando o transporte estiver disponível. Alterações permanentes de especificação exigem revisão do código-fonte; não reescrever expectativas automaticamente para fazer testes passarem.

## Quebra de implementação e modelos fixos — R8

Esta tabela define os modelos de implementação Codex. Não altera a frota de execução Codex/Grok/Antigravity, nem a calibra.

| Ordem | Entrega / arquivos principais | Modelo e esforço | Dependência / aceite |
|---|---|---|---|
| 1 | Tipos v2, compatibilidade, schema e dicionário; `report.rs` novo | `gpt-5.6-luna`, medium | R1 e testes de serialização |
| 2 | Mapeador OTLP único e atributos; `telemetry.rs` | `gpt-5.6-luna`, medium | 1; R2 e testes de paridade |
| 3 | Acumulador, finalizador e progresso; `runner.rs` | `gpt-5.6-terra`, medium | 1; R3/R4 com relógio falso e falhas reais limitadas |
| 4 | Saída CLI, recibos isolados, URL e retenção; `main.rs` | `gpt-5.6-luna`, medium | 2 e 3; R5/R6 e testes do executável |
| 5 | Verificador, interface de visibilidade e backend falso | `gpt-5.6-luna`, medium | 2 e 4; matriz R7 completa |
| 6 | Integração e revisão das transições de falha | `gpt-5.6-terra`, medium | 1–5; checks abaixo e rastreabilidade R1–R8 |
| 7 | README, exemplos e evidências de validação | `gpt-5.6-luna`, low | 6; comandos e limites conferidos no código final |

Automação determinística de testes, schema e comparação usa Rust/Cargo sem LLM. Etapas 2 e 3 podem correr em paralelo depois de fechar os tipos da etapa 1; cada worker tem arquivos próprios. Sem troca automática de modelo. Se uma tarefa exceder a capacidade/configuração disponível, registrar o bloqueio e reduzir sua divisão antes de considerar um modelo maior.

Validação final: `cargo fmt --check`, Clippy do workspace com `-D warnings`, `cargo test --workspace`, `cargo doc --workspace --no-deps`, `cargo guardrails` e matriz CI Windows/Linux/macOS. Registrar execução local e CI separadamente. Não mudar core/client, instaladores ou hooks. Execução live, consulta a backend real e instalação ativa ficam com evidências próprias; os testes offline não as substituem.

## Evidência da implementação

Em 2026-09-13, no Windows: `cargo guardrails` passou (formatação, Clippy do workspace, testes de biblioteca/integração, conformance, documentação e tamanho do cliente). O laboratório possui 64 testes de biblioteca/integração aprovados, incluindo schema JSON, relatório parcial em HTTP 500, recibos por repetição, cancelamento cooperativo, inicialização sem executável real, isolamento de IDs e metadados OTLP. O teste de distribuição confirmou que o laboratório não é uma dependência do produto instalado. A matriz Linux/macOS/Windows está configurada; não há resultado remoto de CI nesta evidência. Nenhuma campanha real, consulta de backend real ou instalação foi executada.
