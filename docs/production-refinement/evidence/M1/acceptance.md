# Contrato de aceite M1

Estado: accepted pelo coordenador após recibo do executor e verificações independentes dos hooks codificados e API de dashboards.
Responsável pelo aceite: coordenador. Executor: m1_inventory, gpt-6-luna medium solicitado.

## Critérios

1. Identificar checkout/base e arquivos ativos por caminho, tamanho e SHA-256; comparar manifesto sem concluir origem dos binários por suposição.
2. Identificar processos, configuração de serviços e startup; distinguir falha de sondagem de ausência de processo.
3. Identificar configuração e registros de bridge nos quatro clientes, preservando terceiros; limitações de precedência explícitas.
4. Mapear receptor/exportador do Collector Windows e destino WSL.
5. Identificar implantação SigNoz, versão, imagens/digests e volumes persistentes.
6. Inventariar templates e dashboards publicados acessíveis; separar fonte, publicação e gerador não identificado.
7. Salvar evidência sanitizada e próxima ação, sem mudar runtime, hooks, implantação ou credenciais.

Desconhecidos de diagnóstico podem avançar a M2 desde que não sejam apresentados como verificados. Coleta incompleta de um item obrigatório mantém M1 aberto.

Resultado: critérios 1–7 satisfeitos para inventário. Evidência: inventory-01.md. Divergência do executor sobre ausência de hooks Claude rejeitada e corrigida após decodificação. Precedência efetiva, origem dos binários e causas de perda não são aceitas como conhecidas; seguem para M2. Runtime permanece inalterado por esta campanha.
