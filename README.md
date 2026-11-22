# WinTint

Personalização do Windows feita com Tauri 2, Rust e TypeScript. Cada recurso é um módulo que pode ser ligado e desligado:

- **Dock:** barra com apps e widgets, que pode substituir a barra de tarefas.
- **Windows:** modo claro ou escuro e cor de destaque do sistema, seguindo o tema da dock ou escolhidos à mão. Seguindo a dock, o destaque é a cor mais viva da barra, ou de um detalhe quando a barra é cinza ou preta. Antes da primeira mudança, o WinTint guarda os valores originais em `windows-original.json`, e desligar o módulo os devolve.
- **Ajustes do Windows:** mudam o Windows na hora, como o Configurações, e continuam valendo com o WinTint fechado. O WinTint lê cada um do próprio Windows sempre que a página abre, sem guardar nada na configuração.
  - Área de trabalho: papel de parede e seu ajuste, quais ícones aparecem, transparência e animações.
  - Barra de tarefas: alinhamento, pesquisa, como juntar as janelas, Visão de Tarefas, segundos no relógio, ocultar automaticamente, e os apps e arquivos recentes do Iniciar.
  - Janelas e arquivos: ajustar janelas nas bordas, sombras, conteúdo ao arrastar, onde o Explorador abre, extensões, arquivos ocultos e o menu clássico do botão direito, que só vale depois de reiniciar o Explorador pelo botão da página.
  - Mouse e teclado: esquema e tamanho do ponteiro, velocidade, rolagem, precisão aprimorada e a repetição das teclas.
  - Sons do sistema: o esquema de sons, na página Geral.
- **Brilho:** escurece cada tela pela curva de gama que o Windows envia a ela, o que funciona em qualquer monitor, até cerca de metade do brilho. A curva original de cada tela fica em `display-original.json` enquanto ela está escurecida e volta quando o WinTint fecha.

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
- `src-tauri/src/color.rs`: conversões de cor compartilhadas.
- `src-tauri/src/windows_theme/`: módulo Windows, com o modo, a cor de destaque e a restauração do original.
- `src-tauri/src/tweaks/`: ajustes do Windows. O `mod.rs` tem os dois comandos que leem e mudam qualquer ajuste pelo nome, e cada área tem seu arquivo:
  - `effects.rs`: transparência, animações, sombras e o comportamento das janelas.
  - `desktop.rs`: papel de parede e ícones da área de trabalho.
  - `shell.rs`: barra de tarefas, Iniciar, Explorador e o reinício do Explorador.
  - `input.rs`: ponteiro, mouse e teclado.
  - `sounds.rs`: esquema de sons.
- `src-tauri/src/display/`: brilho de cada tela.
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
