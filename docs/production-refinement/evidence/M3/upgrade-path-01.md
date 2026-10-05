# M3 — caminho de upgrade verificado, runbook ainda incompleto

Coordenador, leitura somente, 2026-09-30. Nenhum pull, instalação, migração ou restart executado.

## Evidência fixada

Site oficial commit 2b461c59f3dbeaae2761b7862b099684c9a059ed:

- [Schema](https://github.com/SigNoz/signoz.io/blob/2b461c59f3dbeaae2761b7862b099684c9a059ed/constants/upgradeSchema.json).
- [Algoritmo](https://github.com/SigNoz/signoz.io/blob/2b461c59f3dbeaae2761b7862b099684c9a059ed/app/%28site%29/upgrade-path/utils/upgradeUtils.ts).
- [Guia 0.143](https://signoz.io/docs/operate/migration/upgrade-0-143/).

O schema marca 0.143.0 como parada obrigatória; o algoritmo inclui paradas entre origem e destino mais o alvo. Portanto o plano usa 0.141.1 -> 0.143.0 -> 0.144.0. O schema não contém outra parada nesse intervalo. Versões futuras exigem nova resolução.

O guia exige collector 0.144.11 na parada 0.143.0, processors signozspanmapper e signozllmpricing na pipeline, e novo login por mudança para sessões opaque. Manter JWT requer provider explícito e secret existente; não copiar nem exibir secrets. OpAMP preenche configuração dos processors, mas não os insere na pipeline. Metastore guarda mapping/pricing e requer backup. Mapping padrão move mensagens para atributos GenAI e remove originais; mudar para copy é opção explícita, não preservação automática. Fonte: guia acima.

A versão 0.144.11, não comprovada no primeiro parecer AGY, está agora verificada para 0.143.0 por fonte primária. Isto corrige seu estado de evidência, sem validar o restante daquele parecer.

## Próximos itens do runbook da estação

1. Identificar casting.yaml/Foundry que gerou /home/sam/pours/deployment/compose.yaml e suas customizações; não aplicar instalador genérico sobre a estação.
2. Fixar imagens e digests de cada etapa, inclusive collector/migrator, e comparar configurações efetivas sem imprimir credenciais.
3. Dimensionar backup consistente de Postgres/metastore, ClickHouse e keeper, configurações e imagens atuais; escolher local, estimar espaço e provar restauração isolada.
4. Definir cópia isolada com nomes, volumes, rede e portas diferentes; impedir vínculo a volumes originais e evitar exportação dupla ao backend ativo.
5. Especificar comandos da implantação real para migrações síncronas/assíncronas e verificações entre etapas. Não usar executável legado presumido.
6. Verificar login, API keys, 18 dashboards, alertas/views e queries que referenciam atributos movidos; testar ingestão nativa e bridge separadamente.
7. Só promover após ensaio e registrar identidade antes/depois. Recuperar dados e versões compatíveis em conjunto se a migração exigir restauração.

Esses itens ainda não estão resolvidos; documento não é runbook pronto para execução. M3 permanece in_progress.
