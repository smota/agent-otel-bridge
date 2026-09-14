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

Estado: medição atualizada nesta rodada. Os contratos vigentes estão em [production-path-round.md](../dev/fleet-smoke/production-path-round.md): parser puro >50.000 inputs/s, transformação separada e contexto sem filesystem ou espera por refresh no caminho do evento. O benchmark misto é diagnóstico histórico. Tempo interno do hook e ciclo externo de processo são reportados separadamente; aprovação dos microbenchmarks não certifica capacidade do daemon. A perda sob estresse segue aberta em RES-004. Não usar retry automático do teste para esconder perda de eventos.

## RES-003 — Validar comandos entre aspas no executor Antigravity

Na delegação de 2026-09-14, Antigravity Flash Low relatou falha do interceptor PreToolUse antes de conseguir ler ou editar arquivos: o runner não reconheceu o caminho entre aspas do hook. Uma tentativa com prefixo `call` não apareceu no erro retornado; isso não comprova que o runner recarregou a configuração. A alteração temporária foi desfeita. A correção do gerador de comandos deve cumprir o contrato de aspas, mas seus testes não comprovam integração live com esse executor. Antes da próxima release, verificar configuração efetivamente carregada e passagem de argumentos ao shell, com caminho com/sem espaços, sem remover hooks de terceiros. Não declarar o Antigravity validado apenas porque a configuração JSON está correta.

## RES-004 — Investigar perdas na admissão IPC sob estresse

Estado: adiado por decisão explícita do usuário em 2026-09-14; não bloqueia o merge desta rodada. O problema permanece aberto, sem correção de causa comprovada e sem aprovação geral de capacidade sob estresse.

Evidência Windows nativa: três tentativas, com dez repetições de 15 segundos por perfil, emissor persistente abrindo uma conexão por evento e 1.000 ofertas/s. A referência teve 217 falhas em 300.000 ofertas (concorrências 1 e 16); afinidade disjunta teve 227/300.000 nos mesmos perfis, sem benefício consistente. A terceira tentativa teve 295/300.000 nas concorrências 4 e 8; a mudança de perfis impede interpretá-la como regressão contra a referência. As falhas observadas ocorreram na espera pelo pipe ou no deadline. Todos os IDs com escrita concluída foram entregues; nenhum ID de envio falho apareceu no coletor.

Nos cenários de processo por hook, incluindo o controle adicional de concorrência 4, os 15.692 eventos admitidos foram entregues. Ofertas não admitidas pelo gerador foram contabilizadas separadamente; isso não comprova capacidade para toda a carga oferecida. Um trace de 60 segundos teve 31/31 spans e parentesco conferidos via SigNoz MCP. Os microbenchmarks e checks obrigatórios passaram, sem encerrar esta pendência.

Próxima investigação: capturar evidência de escalonamento e disponibilidade do pipe no Windows, medir o custo da instrumentação e variar uma dimensão por vez antes de escolher uma alteração de transporte. Preservar fail-open, watchdog, limites de memória e instalação ativa. Critérios de aceite: comparação reproduzível no mesmo workload, contabilidade por estágio e por ID, nenhuma perda omitida por retry, não regressão dos contratos vigentes. Não há nova arquitetura ou execução autorizada por este item.

Automação e metodologia: [plano da rodada](../dev/fleet-smoke/stall-round-plan.md) e [controlador](../dev/fleet-smoke/stall_round.py). Dados brutos permanecem temporários; este registro preserva a decisão e o resumo necessário para retomar a investigação.
