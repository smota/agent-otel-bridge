# M3 — tentativa 01, não aceita

Executor solicitado: AGY gemini-3.8-flash-medium, --mode plan, --print-timeout 180s. Processo observado PID 17432 criado 2026-09-30 22:30:43 +02; sessão 42867 encerrada com código 0. Saída declara oito chamadas search_web, sem leitura de código, arquivos ou migrações. Modelo observado não foi retornado independentemente pelo CLI; ID acima é o solicitado.

Contrato: ler fontes oficiais fixadas, rastrear mapping/pricing até armazenamento/queries, distinguir evidência e hipótese. Resultado: failed no aceite de pesquisa profunda, apesar de processo concluído.

## Defeitos concretos

- Alegou filtro gen_ai.system/gen_ai.operation.name no PR #12967. Verificação independente do coordenador via API GitHub /pulls/12967/files mostrou mudança para `gen_ai.request.model EXISTS` nos painéis LLM. Merge commit: 5a1be607450f594df935067dbf305cf2127ef883.
- Recomendou `gen_ai.system = agent-otel-bridge` sem comprovar semântica. Recomendação rejeitada; não fabricar identidade do provedor para satisfazer UI.
- Usou signoz_index_v2 como fluxo de armazenamento atual, sem inspeção; instalação inventariada foi consultada em distributed_signoz_index_v3.
- Afirmou versão obrigatória de collector 0.144.11 e migração via signoz-schema-migrator sem fonte de código verificada; guia oficial informa migrator integrado a partir de 0.113. Não usar essa recomendação de upgrade.
- Alegou GA a partir de remoção de feature flag, sem fonte de status do produto. Documentação consultada descreve Beta.
- Afirmações sobre preservação de atributos, null/zero, licensing e preço por unidade não foram demonstradas por código. Permanecem não verificadas, não fatos de arquitetura.

## Decisão

Não aplicar recomendações técnicas desta tentativa. Preservar apenas as pistas oficiais já registradas em research-leads-01.md. Executar fallback previamente autorizado gpt-6.1-sol medium via Codex, com leitura de arquivos fonte fixados, não repetir buscas amplas. Uma tentativa de fallback; se incompleta, salvar lacunas sem inventar resultado.

Fontes da contraprova: https://github.com/SigNoz/signoz/pull/12967/files e https://signoz.io/docs/operate/migration/upgrade-standard/.
