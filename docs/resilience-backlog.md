# Backlog de resiliência e recuperação de telemetria

## RES-001 — Avaliar fila recuperável e integração opcional com serviço existente

Estado: adiado por decisão do usuário em 2026-09-14. Não bloqueia a correção de propagação nativa de TRACEPARENT. Não há decisão de implementar memória compartilhada, guardião ou mensageria própria.

Objetivo: avaliar recuperação de eventos durante indisponibilidade, lentidão ou reinício, preservando o fail-open e os limites de desempenho do hook.

Comparar antes de escolher:

- Reforçar a fila limitada e desacoplar ingestão/exportação do daemon existente.
- Integrar opcionalmente um serviço/coletor ou solução de mensageria existente; manter funcionamento sem esse serviço e explicitar onde ocorre a aceitação dos eventos.
- Reutilizar biblioteca de IPC/memória compartilhada, avaliando ciclo de vida e custo para processos curtos.
- Implementar mecanismo próprio somente se houver necessidade não atendida e benefício demonstrado.

Questões para decisão:

- Recuperação de contexto versus recuperação de eventos; um mapa PID/PPID não substitui uma fila nem prova parent span.
- Quais falhas cobrir: envio falhou cedo, envio aceito seguido de crash, coletor lento, saturação, queda do worker, queda conjunta e reboot.
- Retenção em RAM versus persistência opcional: memória transitória não garante recuperação após reboot.
- Limites de bytes, eventos, idade, recursos e política de descarte; confirmação, IDs estáveis, duplicatas e reordenação.
- Impacto saudável e degradado, tamanho do hook, dependências e operação em Windows/Linux/macOS.
- Custo operacional de mais um processo ou serviço, permissões, instalação, atualização e rollback.

Saída esperada: comparação fundamentada de alternativas e experimento limitado antes de adotar arquitetura nova. Medir o ciclo completo de hooks novos, não extrapolar benchmarks de produtores persistentes. Os exemplos de dimensionamento anteriores são hipóteses, não medições nem configuração aprovada.

Referência: [avaliação de memória compartilhada](shared-memory-resilience-assessment.md). A recomendação experimental desse documento é uma alternativa de pesquisa, não compromisso de implementação.

## Prioridade atual

Corrigir e validar a propagação de contexto hook → IPC → daemon → OTLP, mantendo o transporte existente e sem incluir fila recuperável, guardião, shared memory ou serviço adicional nessa entrega. A campanha de smoke permanece um trabalho separado, retomado após os pré-requisitos próprios.

## RES-002 — Fechar lacunas de medição e aceitação dos SLAs

Estado: pendente de diagnóstico. O benchmark existente de criação de processo mede o ciclo externo completo, enquanto o contrato também exige cliente <1 ms. O benchmark combinado de parsing/conversão deve comprovar >50.000 spans/s; sucesso do `cargo guardrails` não implica esses SLAs aprovados. Esclarecer instrumentação e comparar baseline/candidato sem reduzir os requisitos. Separar também perda em carga concorrente por fail-open de erro de associação de contexto. Não usar retry automático do teste para esconder perda de eventos.
