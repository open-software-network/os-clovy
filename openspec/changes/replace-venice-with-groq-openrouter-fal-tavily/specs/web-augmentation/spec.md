# Spec Delta

## Purpose

Preservar busca e extração de páginas pelas ferramentas do Clovy usando Tavily sem dependência ou fallback Venice.

## ADDED Requirements

### Requirement: Busca Tavily
O sistema SHALL executar busca por Tavily e retornar resultados normalizados com títulos, URLs e conteúdo útil no contrato das ferramentas existentes.

#### Scenario: Busca com resultados
- **WHEN** o usuário ou agente autorizado pesquisa um tema
- **THEN** recebe resultados Tavily utilizáveis e identificados sem invocar Venice ou inventar resultados
- Test: integration — parser de search, erros e boundary das ferramentas; prova externa autorizada.

### Requirement: Extração Tavily
O sistema SHALL extrair páginas por Tavily e fornecer seu conteúdo útil com status de sucesso/erro por URL, respeitando proteções de URL e limites vigentes.

#### Scenario: Extrair página
- **WHEN** uma URL pública permitida é solicitada
- **THEN** a extração retorna conteúdo da URL no fluxo existente ou erro útil se falhar, sem placeholder nem fetch a destino protegido
- Test: integration — extract com successful/failed results e URLs/boundaries; prova externa autorizada.

### Requirement: Erros e privacidade web
O sistema SHALL preservar erros, limites e metering existentes de web sem expor credenciais ou declarar privacidade herdada de Venice.

#### Scenario: Chave inválida ou URL proibida
- **WHEN** Tavily rejeita a credencial ou a URL não é permitida
- **THEN** o sistema rejeita a ação com erro útil, não retransmite a chave a outro serviço e não apresenta conteúdo fabricado
- Test: integration — autenticação/URL/erro/settlement exercitados em sandbox sem requisição paga.
