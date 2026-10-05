# M3 — pistas iniciais para pesquisa upstream

Estado: preliminary; não fecha M3. Coleta pelo coordenador enquanto M2 ocorre, sem executor adicional.

Fonte oficial consultada em 2026-09-30: https://github.com/SigNoz/signoz/releases

- Alvo estável observado: v0.144.0 (2026-09-29, commit curto 47dd1fa). Instalado: v0.141.1 conforme M1.
- v0.143.0 registra adoção de GenAI semconv para ai-o11y (#12905), processors habilitados por padrão (#12912), atualização do dashboard (#12875) e remoção de feature flag (#12947).
- v0.144.0 registra filtro do overview para spans gen_ai e alteração de atributos de mensagens (#12967), além de migração de quick filters (#12964).
- Remoção de backend de dashboards v1 (#12932) também exige inspeção de compatibilidade; não concluir que afeta os templates v2 sem comparação.

Consequência: a pesquisa profunda deve examinar a implementação AI Observability além de MCP, skills e exportação nativa de clientes. A leitura inicial de páginas gerais não cobria essa camada. Os itens acima são pistas de release, não comprovação do funcionamento interno, disponibilidade por edição, nem caminho seguro de upgrade.

Próximo executor M3: gemini-3.8-flash-medium via AGY. Fixar commits e rastrear código, configurações e testes destes itens. Sol high prepara o runbook de migração WSL a partir da implantação real. Nenhum pull, upgrade ou migração executado.

## Documentação técnica descoberta

- https://signoz.io/docs/ai-observability/ : descreve a camada self-hosted, mapping/pricing, processors signozspanmapper e signozllmpricing; preços por modelo precisam ser configurados no OSS. A documentação informa custo derivado em signoz.gen_ai.usage.tokens.cost. Conteúdo obtido na busca; abertura direta falhou neste tool, portanto confirmar via fonte Markdown/GitHub antes de aprofundar.
- https://signoz.io/docs/ai-observability-explorer/ : diferencia consultas por span e totais por trace; investigar semântica de agregação e risco de dupla contagem.
- https://signoz.io/docs/operate/upgrade/ : abertura direta confirmou exigência de verificar Upgrade Path Tool e paradas obrigatórias, fixar versões e aguardar migrações antes de avançar. Isso ainda não determina o caminho exato 0.141.1 -> 0.144.0.
