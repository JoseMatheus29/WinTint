# WinTint

Personalização do Windows feita com Tauri 2, Rust e TypeScript. Cada recurso é um módulo que pode ser ligado e desligado. Por enquanto existe um: a dock.

## Comandos

Instalar as dependências:

```bash
npm install
```

Rodar em desenvolvimento:

```bash
npm run tauri dev
```

Gerar o executável em `src-tauri/target/release/wintint.exe`:

```bash
npm run tauri build -- --no-bundle
```

## Configuração

Para abrir a tela de personalização, clique com o botão direito na dock, use o ícone na bandeja do sistema ou abra o programa de novo, o que também funciona com todos os módulos desligados.

As mudanças valem na hora e ficam salvas em `%APPDATA%\com.jmatheus.wintint\config.json`, com uma seção para cada módulo. Na primeira execução, o WinTint importa as configurações do Dock JM, o nome anterior do app, de `%APPDATA%\com.jmatheus.dockjm\dock.json`, sem alterar esse arquivo.

## Estrutura

- `src-tauri/src/lib.rs`: inicialização, bandeja, janela de personalização e comandos do app.
- `src-tauri/src/config.rs`: leitura do `config.json` e importação do Dock JM.
- `src-tauri/src/win.rs`: chamadas à API do Windows usadas pelos módulos.
- `src-tauri/src/dock/`: módulo da dock.
  - `mod.rs`: janela da dock e monitoramento do cursor e das janelas.
  - `config.rs`: opções da dock.
  - `widgets.rs`: dados de volume, rede, bateria, CPU, memória e música.
- `src/config.ts`: tipos da configuração.
- `dock.html`, `src/dock/`: página da dock.
  - `main.ts`, `styles.css`: dock, efeito de ampliação e cliques.
  - `widgets.ts`: widgets dentro da dock.
  - `appearance.ts`: temas prontos, padrões e aplicação do visual.
- `settings.html`, `src/settings/`: tela de personalização.
  - `settings.ts`, `settings.css`: páginas, controles e prévia da dock.
  - `palette.ts`: cores da tela, calculadas a partir do tema da dock.
