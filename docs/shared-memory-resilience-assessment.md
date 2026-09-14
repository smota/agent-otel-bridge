# Memória compartilhada para recuperação transitória

Estado: avaliação técnica, não plano de implementação nem aprovação de mudança do transporte. Separada da campanha em `dev/fleet-smoke/first-live-run-plan.md`. Não foram executados benchmarks nesta avaliação.

Decisão posterior do usuário (2026-09-14): adiar fila/recuperação e avaliar também integração opcional com serviço existente. Acompanhar em [RES-001](resilience-backlog.md). Guardião e memória compartilhada não fazem parte da correção imediata de contexto.

## Conclusão

É viável investigar memória compartilhada como reserva de eventos quando o pipe falha cedo. O impacto no caminho saudável pode ser pequeno se nenhum mapeamento for aberto ou acessado nesse caminho. Não é custo zero: código adicional pode alterar tamanho e inicialização; um processo que mantenha a região viva consome recursos. A magnitude só pode ser estabelecida com o hook real, iniciado novamente a cada evento.

| Modo | Trabalho adicional no caminho saudável | Cobertura |
|---|---|---|
| Fallback sob falha de pipe | Desvio de controle e eventual identidade de mensagem; sem abrir/mapear região | Falhas de envio detectadas antes do deadline |
| Escrita em pipe e região | Mapeamento, cópia e sincronização em todo evento | Mais retenção, mas exige deduplicação e confirmação |
| Região como transporte principal | Mapeamento, publicação e sinalização em todo evento; elimina parte do trabalho do pipe | Topologia mais uniforme, porém migração maior |

Enviar ao pipe com sucesso não confirma recepção pelo daemon ou exportação. Fallback acionado somente por erro não recupera um evento perdido após sucesso da escrita. Timeout também pode deixar resultado ambíguo: repetir pelo outro canal pode duplicar. Uma identidade estável deve nascer antes do primeiro envio e ser preservada no replay; deduplicação em memória não garante exactly-once após reinício.

## Ciclo de vida e limites

No Windows, uma região apoiada no pagefile precisa de handles/views vivos. Se todos os processos que a mantêm terminarem, não há reserva sobrevivente garantida. Um guardião pequeno separado do worker/exportador pode manter a região durante reinícios deste worker. Ele não protege contra queda conjunta, reboot ou falta de energia. Um arquivo de backing mudaria o contrato de persistência e desempenho.

O guardião deve criar e inicializar a região fora do hot path; os hooks apenas abrem uma região existente no fallback. Restringir acesso ao usuário/sessão apropriados; não presumir que um nome Global funciona sem privilégios ou que Local atende execuções em outra sessão. Unix requer backend de recursos e limpeza próprios, mantendo API e layout portáveis em Rust, sem scripts ps1.

Memória compartilhada apoiada no pagefile não significa RAM permanentemente residente. Bloquear páginas tem limites e custo para o sistema; pré-aquecer uma região no guardião não elimina todo custo de mapeamento no novo processo hook.

## Segurança de concorrência em Rust

O watchdog pode matar um produtor durante a publicação. A fila precisa distinguir slot reservado, parcialmente escrito e publicado. Um produtor morto não pode bloquear os demais. Expiração sozinha não autoriza reutilizar um slot: um processo apenas pausado pode voltar a escrever. Reuso exige protocolo seguro de posse/geração e comprovação de término quando necessário, sem espera no hook.

Usar layout binário versionado, offsets em vez de ponteiros e limites fixos; não colocar Vec, String, referências ou Mutex padrão entre processos. Sincronização precisa ter garantias documentadas para o alvo, com alinhamento e ordenação corretos. Lock-free não implica limite de tempo por chamada: tentativas de reserva devem ser limitadas.

Fila cheia, segmento ausente, incompatível ou sem permissão devem manter fail-open. Definir limite de payload e política para eventos maiores, sem truncamento silencioso. Um mapa PID/PPID não substitui essa fila nem prova causalidade de spans.

## Experimento recomendado antes da escolha

Comparar release atual, envelope sem shared memory, fallback habilitado com pipe saudável, fallback com falha imediata, e falha lenta. Medir processos novos, tamanhos reais de payload, p50/p95/p99, watchdog, CPU, bytes do executável e recursos do guardião. Incluir pressão de memória e concorrência. Não transportar resultados de benchmarks com publishers persistentes para este caso.

Manter SLA cliente <1 ms, watchdog 3 ms e binário <300 KB. O IPC atual já pode consumir todo o orçamento de 3 ms: não acrescentar fallback depois desse deadline. Orçamento deve ser compartilhado desde o início; a reserva exata para cada fase é resultado do experimento. Pode haver recuperação em falha imediata e descarte em falha lenta.

Testar morte do produtor antes/durante/depois da publicação, pausa longa seguida de retorno, reinício do consumidor, fila cheia, identidade de processo reutilizada, versão incompatível e entrega duplicada. Separar evento publicado na região, consumido, aceito pelo coletor e visível no SigNoz. Retenção até aceitação cobre mais falhas, mas aumenta capacidade e protocolo necessários.

Primeira candidata: fallback sob falha precoce, região limitada mantida por guardião e publicação com tempo limitado. Evitar dual-write inicialmente. Mesmo com essa opção, desacoplar ingestão e exportação no daemon continua necessário; shared memory não corrige bloqueio por HTTP lento.

## Fontes primárias

- [Microsoft: named shared memory e ciclo de vida](https://learn.microsoft.com/en-us/windows/win32/memory/creating-named-shared-memory)
- [Microsoft: sincronização interprocesso com Interlocked](https://learn.microsoft.com/en-us/windows/win32/sync/interlocked-variable-access)
- [Microsoft: VirtualLock, limites e efeitos no sistema](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtuallock)
- [Rust: atomic memory ordering](https://doc.rust-lang.org/stable/core/sync/atomic/)
- [Eclipse iceoryx2: middleware Rust de shared memory](https://github.com/eclipse-iceoryx/iceoryx2)

Iceoryx2 é referência para avaliar protocolos e recuperação. Sua adoção no cliente não está recomendada sem verificar dependências, inicialização, operações de filesystem e tamanho final frente ao SLA deste repositório.
