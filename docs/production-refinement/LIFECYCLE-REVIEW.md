# Revisão de cobertura do ciclo de vida dos agentes

Estado: discussão registrada; decisão e implementação pendentes. Solicitada por Samuel em 2026-10-06 após habilitar os hooks Codex. Não ampliar a configuração durante o teste final dos três hooks existentes.

## Questão de produto

SubagentStart/SubagentStop e SessionEnd devem fazer parte da observabilidade? Recomendação preliminar: sim, onde o harness fornece eventos e identificadores confiáveis. São necessários para investigar delegação e encerramento; a mera presença do nome na configuração não comprova cobertura.

| Sinal a avaliar | Pergunta do operador | Contrato a definir e provar |
| --- | --- | --- |
| SessionStart/SessionEnd | Quais sessões começaram, terminaram ou ficaram sem encerramento observado? | Identidade, retomada, razão, limites de entrega em crash/kill; ausência de SessionEnd não equivale a sessão ativa |
| SubagentStart/SubagentStop | Quem delegou para quem e qual filho terminou? | IDs de pai/filho, correlação, profundidade, duração e eventos ausentes; impedir dupla contagem com spans do próprio filho |
| Stop | Qual turno terminou? | Separar término de turno, sessão e tarefa; investigar primeiro os dois Stop reais do Grok |
| Interrupt/falha | Onde a operação foi interrompida? | Distinguir interrupção observada, erro da ferramenta e sucesso da tarefa; disponibilidade por cliente/superfície |

A documentação atual do [Codex](https://learn.chatgpt.com/docs/hooks#matcher-patterns) lista esses eventos e limita SessionEnd ao motivo `other`. Informa também que os hooks de subagentes usam a sessão do pai. Portanto, session_id sozinho não deve ser assumido como identidade do filho. Documentação consultada em2026-10-06; validar payloads e comportamento na versão instalada antes da implementação.

## Momento e responsáveis

1. **M2, coleta:** matriz AGY/Claude/Codex/Grok × CLI/Desktop/IDE usados × eventos suportados/configurados/observados. AGY `gemini-3.8-flash-low` para inventário; Grok `grok-4.7-build-fast` low para diagnóstico de duplicação. Capturar somente tipos, IDs, flags e timestamps necessários.
2. **M4, contrato:** Grok `grok-4.7` como consultor principal; consolidar origem, identidade, parentesco, deduplicação, incompletude e significado de cada fim. Codex Sol medium para revisão crítica delimitada. Sem equivalências automáticas entre nomes de clientes.
3. **M6, implementação:** AGY `gemini-3.8-flash-medium`, um escritor, revisão Grok Build Fast. Verificar modelo de evento, wire tags, parser, spans e projeção do instalador; não basta acrescentar chaves JSON. Preservar compatibilidade, terceiros, fail-open e orçamento nativo; revisão normal de confiança após mudanças.
4. **M5/M8, dashboards:** desenho consome contrato M4; validação consome evidências M6. Visões de sessão e delegação devem mostrar cobertura e dados ausentes. Não contar sucesso por Stop ou SessionEnd, nem confundir spans com tarefas concluídas.

## Critérios de fechamento

- Uma delegação real com pai e filho correlacionados e términos distintos, no mínimo na superfície priorizada; limites das demais explicitados.
- Uma sessão encerrada normalmente e uma interrompida, sem inventar evento de fechamento quando não recebido.
- Nenhuma dupla contagem entre hook do pai, evento do filho e telemetria nativa; regra documentada e testada.
- Reinstalação preserva projeção e terceiros, com custo do shell medido separadamente do cliente nativo.
- Cobertura e lacunas visíveis no dashboard apropriado. Aprovação de nomenclatura/semântica em M4 antes de ativação geral.

Esta discussão não está resolvida pelo teste Codex de três hooks. Pi permanece fora do escopo; somente subscrições, no máximo duas tentativas por unidade, com evidência persistida antes de avançar.
