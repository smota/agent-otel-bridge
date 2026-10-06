# M2 tentativa 02 — contrato e intenção

Data: 2026-10-06. Estado: preparação iniciada; nenhum daemon iniciado nesta tentativa.
Base: 67ef649. Coordenador é único escritor. AGY gemini-3.8-flash-low prepara evidência somente leitura; Grok grok-4.7-build-fast revisa; AGY gemini-3.8-flash-medium executa receita isolada se demonstravelmente segura.

Escopo: revalidar identidades de binários, processos/pipes e suporte a isolamento no build ativo; capturar stdout/stderr, código de saída, identidade e limpeza em teste delimitado. Não instalar, alterar hooks, reiniciar runtime ativo nem exportar prompts. Não presumir correspondência entre fonte HEAD e binário ativo.

Aceite da tentativa: evidência suficiente para executar teste isolado ou impedimento demonstrado com próxima ação. M2 só fecha com cobertura dos quatro clientes; esta tentativa não a presume.

Limites: subscrições existentes; uma consulta auxiliar por vez; até duas tentativas; saída bruta temporária fora do Git, síntese saneada nesta pasta. Antes do teste registrar executável/hash, variáveis, pipe, endpoint, timeout e término do PID criado. Na retomada reconciliar processos antes de repetir.

## Preparação e revisão

- AGY Flash low: tentativa1 retornou SUCCESS com resposta vazia e denied_actions RunCommand; não aceita. Tentativa2 recebeu digest e entregou parecer (conversa 81bc5042 da primeira tentativa; IDs completos nos recibos locais). Sem acesso a ferramentas na tentativa2; fatos foram coletados pelo coordenador.
- Grok Build Fast: sessão 01a10e9c-cf16-7360-be66-27fac35ff772; parecer final exige mudanças, não aprovação irrestrita. Aceitos ambiente mínimo, job Windows kill-on-close, processo suspenso antes da associação, pipe com PID verificado, timeout e reconciliação.
- Correções do coordenador: endpoint deve ser URL base sem /v1/traces; defaults reais são agent-otel e agy-otel. QuotaWorker pode consultar imediatamente: intervalo60s não demonstra ausência de leitura. Home temporário limita os providers da fonte inspecionada, mas não é sandbox do token Windows. Nenhuma alegação de isolamento universal de rede/arquivos.
- Teste autorizado é diagnóstico local com eventos sintéticos mínimos. Não usar dados de sessões reais. Observar pipe exclusivo e receptor local; resultado não certifica proveniência do build nem integração real dos clientes.
- Identidade ativa: versão0.5.2, bridge SHA256 24C73837129C674D80DF727B85D04915952FC9C73B9BFFB451E6A531497C6E9D; hook E776F732E2773F7AEF1A70C86F8B6057824AA8070082B902585876690E05E030. Nenhum processo/pipe bridge observado. Strings dos controles presentes no binário; comportamento será verificado.
- Intenção: supervisor em TEMP/aob-m2-20261006/probe.py; stdout/stderr e receipt.json em subdiretório único. Pipe agent-otel-m2-UUID, sink127.0.0.1:porta efêmera, ambiente mínimo, idle120s, deadline20s. Executar binário ativo em primeiro plano; encerramento por frameFF ou job; nunca comando start/stop global. Não instalar candidato.

## Extensão delimitada: launcher e hook nativo

O teste foreground encerrou com exit0,4 spans aceitos e limpeza confirmada. Próxima unidade: mesma contenção em job e ambiente, invocar start somente com pipe privado e receptor local. O launcher pode encerrar antes do daemon: verificar que o PID servidor pertence ao job criado (IsProcessInJob), nunca aceitar PID externo. Invocar o hook canônico com quatro payloads sintéticos e --client explícito; capturar respostas. Isso testa launcher e binário hook, não a configuração real dos harnesses. Deadline20s e encerramento do job inteiro; observar ausência de pipes antes e depois. Script launcher-probe.py, recibo em diretório launcher-UUID. Não executar start no ambiente normal.
