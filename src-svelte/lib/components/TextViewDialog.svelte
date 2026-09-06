<!-- 【新手导读】只读文本查看对话框：标题 + 等宽文本区 + 关闭按钮。
     点击遮罩空白处也可关闭。用于查看 config.xml 等生成文件的内容。 -->
<script lang="ts">
  let { title, text, onclose }: { title: string; text: string; onclose: () => void } = $props();
</script>

<div class="modal-backdrop" role="presentation"
     onclick={(e) => { if (e.target === e.currentTarget) onclose(); }}>
  <div class="modal">
    <div class="modal-title">
      <span>{title}</span>
      <button class="btn" onclick={onclose}>关闭</button>
    </div>
    <div class="modal-body">
      <pre class="text-view">{text}</pre>
    </div>
  </div>
</div>
