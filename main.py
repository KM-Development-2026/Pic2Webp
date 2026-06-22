import os
import re
import sys
import threading
import queue
import subprocess
import customtkinter as ctk
from tkinterdnd2 import TkinterDnD, DND_FILES
import archive_processor

# アプリケーションで使用するパース関数
def parse_drop_files(data):
    """
    TkinterDnD2のドロップデータからファイルパスのリストを取り出します。
    スペースを含むパスは中括弧 {} で囲まれるか、あるいはスペース区切りの文字列になるため、
    正規表現で適切に分割してパースします。
    """
    # Windowsでドロップされた複数ファイルは、スペースが含まれると {} で囲まれる
    pattern = r'\{([^}]+)\}|([^\s{}]+)'
    matches = re.findall(pattern, data)
    paths = []
    for m in matches:
        path = m[0] if m[0] else m[1]
        if path:
            # 円記号をスラッシュに正規化し、ダブルクォーテーションを取り除く
            normalized = path.replace('/', '\\').strip().strip('"')
            paths.append(normalized)
    return paths

class Pic2WebpApp:
    def __init__(self, root):
        self.root = root
        self.root.title("Archive Image WebP Converter")
        self.root.geometry("650x650")
        self.root.minsize(550, 600)
        
        # キューを作成（別スレッドからのUI更新用）
        self.queue = queue.Queue()
        
        # 処理中の状態フラグ
        self.is_processing = False
        self.last_new_temp_dir = None
        self.current_quality = 50
        
        self.setup_ui()
        self.check_queue()
        
    def setup_ui(self):
        # テーマ設定
        ctk.set_appearance_mode("Dark")
        ctk.set_default_color_theme("blue")
        
        # メインフレーム
        self.main_frame = ctk.CTkFrame(self.root)
        self.main_frame.pack(fill="both", expand=True, padx=20, pady=20)
        
        # タイトル
        self.title_label = ctk.CTkLabel(
            self.main_frame, 
            text="Archive Image WebP Converter", 
            font=ctk.CTkFont(size=22, weight="bold")
        )
        self.title_label.pack(pady=(15, 5))
        
        self.desc_label = ctk.CTkLabel(
            self.main_frame,
            text="圧縮ファイル (zip, rar, 7z) をドラッグ＆ドロップして、\n中身の画像を WebP (品質 50%) に一括変換し、再度 zip 圧縮します。\n(画像以外のテキストやリンクなどのファイルは自動的に除外されます)",
            font=ctk.CTkFont(size=12),
            text_color="#a0a0a0"
        )
        self.desc_label.pack(pady=(0, 20))
        
        # D&Dエリア
        self.dnd_frame = ctk.CTkFrame(
            self.main_frame,
            height=130,
            fg_color="#212121",
            border_color="#1f538d",
            border_width=2,
            corner_radius=10
        )
        self.dnd_frame.pack(fill="x", padx=15, pady=10)
        self.dnd_frame.pack_propagate(False)
        
        self.dnd_label = ctk.CTkLabel(
            self.dnd_frame,
            text="ここに圧縮ファイルをドラッグ＆ドロップしてください\n(複数ファイルを同時に処理可能)",
            font=ctk.CTkFont(size=14, weight="bold")
        )
        self.dnd_label.pack(expand=True)
        
        # D&Dイベントの登録
        self.dnd_frame.drop_target_register(DND_FILES)
        self.dnd_frame.dnd_bind('<<Drop>>', self.on_drop)
        self.dnd_frame.dnd_bind('<<DragEnter>>', self.on_drag_enter)
        self.dnd_frame.dnd_bind('<<DragLeave>>', self.on_drag_leave)
        
        # 設定フレーム
        self.settings_frame = ctk.CTkFrame(self.main_frame, fg_color="transparent")
        self.settings_frame.pack(fill="x", padx=15, pady=10)
        
        # 品質スライダー
        self.quality_label = ctk.CTkLabel(
            self.settings_frame,
            text="WebP 圧縮品質: 50%",
            font=ctk.CTkFont(size=12, weight="bold")
        )
        self.quality_label.pack(anchor="w", pady=(0, 5))
        
        self.quality_slider = ctk.CTkSlider(
            self.settings_frame,
            from_=5,
            to=100,
            number_of_steps=95,
            command=self.on_slider_change
        )
        self.quality_slider.set(50)
        self.quality_slider.pack(fill="x", pady=(0, 5))
        
        # 進捗フレーム
        self.progress_frame = ctk.CTkFrame(self.main_frame, fg_color="transparent")
        self.progress_frame.pack(fill="x", padx=15, pady=5)
        
        self.status_label = ctk.CTkLabel(
            self.progress_frame,
            text="処理待機中...",
            font=ctk.CTkFont(size=12)
        )
        self.status_label.pack(anchor="w")
        
        self.progress_bar = ctk.CTkProgressBar(self.progress_frame)
        self.progress_bar.set(0)
        self.progress_bar.pack(fill="x", pady=5)
        
        # ログ表示エリア
        self.log_label = ctk.CTkLabel(
            self.main_frame,
            text="処理ログ:",
            font=ctk.CTkFont(size=12, weight="bold")
        )
        self.log_label.pack(anchor="w", padx=15, pady=(5, 0))
        
        self.log_textbox = ctk.CTkTextbox(
            self.main_frame,
            height=160,
            font=ctk.CTkFont(family="Consolas", size=11)
        )
        self.log_textbox.pack(fill="both", expand=True, padx=15, pady=(5, 10))
        self.log_textbox.configure(state="disabled")
        
        # アクションボタンフレーム
        self.btn_frame = ctk.CTkFrame(self.main_frame, fg_color="transparent")
        self.btn_frame.pack(fill="x", padx=15, pady=(0, 10))
        
        self.open_folder_btn = ctk.CTkButton(
            self.btn_frame,
            text="保存先 (new_temp) を開く",
            command=self.open_output_folder,
            state="disabled",
            fg_color="#2ecc71",
            hover_color="#27ae60",
            text_color="#ffffff"
        )
        self.open_folder_btn.pack(side="right", padx=5)
        
        self.clear_log_btn = ctk.CTkButton(
            self.btn_frame,
            text="ログをクリア",
            command=self.clear_log,
            fg_color="#7f8c8d",
            hover_color="#95a5a6",
            text_color="#ffffff"
        )
        self.clear_log_btn.pack(side="left", padx=5)

    def on_drag_enter(self, event):
        if not self.is_processing:
            self.dnd_frame.configure(border_color="#2ecc71") # ドラッグ中は緑色に
            self.dnd_label.configure(text="ここにドロップして処理を開始")
        
    def on_drag_leave(self, event):
        if not self.is_processing:
            self.dnd_frame.configure(border_color="#1f538d") # 離れたら元の青色に
            self.dnd_label.configure(text="ここに圧縮ファイルをドラッグ＆ドロップしてください\n(複数ファイルを同時に処理可能)")

    def on_slider_change(self, value):
        self.current_quality = int(value)
        self.quality_label.configure(text=f"WebP 圧縮品質: {self.current_quality}%")

    def append_log(self, text):
        self.log_textbox.configure(state="normal")
        self.log_textbox.insert("end", text)
        self.log_textbox.see("end")
        self.log_textbox.configure(state="disabled")

    def clear_log(self):
        self.log_textbox.configure(state="normal")
        self.log_textbox.delete("1.0", "end")
        self.log_textbox.configure(state="disabled")

    def set_ui_state(self, state):
        self.quality_slider.configure(state=state)
        self.clear_log_btn.configure(state=state)
        if state == "normal":
            self.dnd_label.configure(text="ここに圧縮ファイルをドラッグ＆ドロップしてください\n(複数ファイルを同時に処理可能)")
            self.dnd_frame.configure(border_color="#1f538d")
        else:
            self.dnd_label.configure(text="現在処理中です。しばらくお待ちください...")
            self.dnd_frame.configure(border_color="#e67e22") # 処理中はオレンジ色に

    def on_drop(self, event):
        if self.is_processing:
            return
            
        file_paths = parse_drop_files(event.data)
        if not file_paths:
            return
            
        self.is_processing = True
        self.set_ui_state("disabled")
        self.open_folder_btn.configure(state="disabled")
        self.progress_bar.set(0)
        
        # 処理用スレッドを起動
        thread = threading.Thread(
            target=self.process_worker,
            args=(file_paths, self.current_quality),
            daemon=True
        )
        thread.start()

    def process_worker(self, file_paths, quality):
        # フィルタリング: zip, rar, 7z のみ
        valid_files = []
        for path in file_paths:
            if not os.path.exists(path):
                continue
            ext = os.path.splitext(path)[1].lower()
            if ext in ['.zip', '.rar', '.7z']:
                valid_files.append(path)
            else:
                self.queue.put(('log', f"【スキップ】 未対応の拡張子です: {os.path.basename(path)}\n\n"))
                
        if not valid_files:
            self.queue.put(('error', "処理可能な圧縮ファイル (zip, rar, 7z) がありませんでした。"))
            return
            
        total_files = len(valid_files)
        success_count = 0
        last_new_temp = None
        
        for i, file_path in enumerate(valid_files):
            filename = os.path.basename(file_path)
            self.queue.put(('status', f"処理中 ({i+1}/{total_files}): {filename}"))
            
            # 各ファイルの処理中の進捗をコールバックで受け取る
            def progress_cb(current, total, msg):
                # ログをキューにプッシュ
                # 進捗割合を計算してプログレスバーに反映
                # i が現在のファイル、current がそのファイル内での進捗
                # 全体の進捗 = (i / total_files) + (current / total / total_files)
                overall = (i / total_files) + ((current / total) / total_files if total > 0 else 0)
                self.queue.put(('overall_progress', overall))
                
            def log_cb(msg):
                self.queue.put(('log', msg))
                
            try:
                out_path = archive_processor.process_single_archive(
                    file_path, 
                    quality=quality, 
                    log_callback=log_cb, 
                    progress_callback=progress_cb
                )
                success_count += 1
                last_new_temp = os.path.dirname(out_path)
            except Exception as e:
                self.queue.put(('log', f"[エラー] {filename} の処理に失敗しました。\n詳細: {str(e)}\n\n"))
                
        self.queue.put(('log', f"【完了】 処理に成功したファイル: {success_count} / {total_files} 件\n"))
        self.queue.put(('done', last_new_temp))

    def check_queue(self):
        try:
            while True:
                task = self.queue.get_nowait()
                event_type = task[0]
                
                if event_type == 'log':
                    self.append_log(task[1])
                elif event_type == 'overall_progress':
                    self.progress_bar.set(task[1])
                elif event_type == 'status':
                    self.status_label.configure(text=task[1])
                elif event_type == 'done':
                    self.is_processing = False
                    self.progress_bar.set(1.0)
                    self.status_label.configure(text="すべての処理が完了しました")
                    self.last_new_temp_dir = task[1]
                    if self.last_new_temp_dir and os.path.exists(self.last_new_temp_dir):
                        self.open_folder_btn.configure(state="normal")
                    self.set_ui_state("normal")
                elif event_type == 'error':
                    self.append_log(f"[エラー] {task[1]}\n\n")
                    self.status_label.configure(text="エラーが発生しました")
                    self.set_ui_state("normal")
                    self.is_processing = False
                
                self.queue.task_done()
        except queue.Empty:
            pass
        finally:
            self.root.after(100, self.check_queue)

    def open_output_folder(self):
        if self.last_new_temp_dir and os.path.exists(self.last_new_temp_dir):
            try:
                # Windowsのエクスプローラーでフォルダを開く
                os.startfile(self.last_new_temp_dir)
            except Exception as e:
                self.append_log(f"[エラー] 保存先フォルダを開けませんでした。\n詳細: {str(e)}\n\n")

def main():
    # ウィンドウ初期化
    root = TkinterDnD.Tk()
    app = Pic2WebpApp(root)
    root.mainloop()

if __name__ == "__main__":
    main()
