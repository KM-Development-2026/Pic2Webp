import os
import shutil
import subprocess
import tempfile
import zipfile
from PIL import Image
import py7zr
import rarfile

def find_rar_tool():
    """
    Rarファイルを展開するための外部ツール (7z.exe または unrar.exe) を検索します。
    見つかった場合はそのフルパスを返し、見つからない場合は None を返します。
    """
    # 1. PATH上のツールを検索
    for tool in ["7z", "7zg", "unrar"]:
        path = shutil.which(tool)
        if path:
            return path

    # 2. Windowsの一般的なインストール先を検索
    common_paths = [
        r"C:\Program Files\7-Zip\7z.exe",
        r"C:\Program Files (x86)\7-Zip\7z.exe",
        r"C:\Program Files\WinRAR\UnRAR.exe",
        r"C:\Program Files\WinRAR\WinRAR.exe",
    ]
    for path in common_paths:
        if os.path.exists(path):
            return path

    return None

def is_image_file(file_path):
    """
    ファイルが画像ファイル（Pillowで開けるか）どうかを判定します。
    """
    # 拡張子での簡易チェック（速度向上のため）
    valid_extensions = {".jpg", ".jpeg", ".png", ".webp", ".bmp", ".tiff", ".gif"}
    ext = os.path.splitext(file_path)[1].lower()
    if ext not in valid_extensions:
        return False

    try:
        with Image.open(file_path) as img:
            img.verify()
        return True
    except Exception:
        return False

def clean_empty_directories(dir_path):
    """
    指定されたディレクトリ配下の空のディレクトリを再帰的に削除します。
    """
    for root, dirs, files in os.walk(dir_path, topdown=False):
        for d in dirs:
            path = os.path.join(root, d)
            if not os.listdir(path):
                os.rmdir(path)

def extract_archive(archive_path, extract_dir, rar_tool=None):
    """
    圧縮ファイルを指定されたディレクトリに展開します。
    """
    ext = os.path.splitext(archive_path)[1].lower()
    
    if ext == ".zip":
        with zipfile.ZipFile(archive_path, 'r') as zip_ref:
            zip_ref.extractall(extract_dir)
            
    elif ext == ".7z":
        with py7zr.SevenZipFile(archive_path, mode='r') as sz_ref:
            sz_ref.extractall(extract_dir)
            
    elif ext == ".rar":
        if not rar_tool:
            raise RuntimeError("RARファイルの展開に必要な外部ツール（7-Zip または WinRAR）が見つかりません。")
            
        # 展開先ディレクトリを作成しておく
        os.makedirs(extract_dir, exist_ok=True)
        
        # ツール名（またはパス）によって引数を切り分ける
        tool_lower = rar_tool.lower()
        if "7z" in tool_lower:
            # 7-Zip を使用して展開
            # x: フォルダ構造維持して展開, -y: すべてはいで応答, -o: 展開先
            cmd = [rar_tool, "x", "-y", f"-o{extract_dir}", archive_path]
        else:
            # unrar / WinRAR を使用して展開
            # x: フォルダ構造維持して展開, -y: すべてはいで応答
            # unrarの展開先は最後にバックスラッシュが必要な場合がある
            cmd = [rar_tool, "x", "-y", archive_path, os.path.join(extract_dir, "")]
            
        subprocess.run(cmd, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    else:
        raise ValueError(f"未対応の圧縮形式です: {ext}")

def convert_images_in_dir(target_dir, quality=50, progress_callback=None):
    """
    ディレクトリ内の画像をWebPに変換し、それ以外のファイルを削除します。
    """
    all_files = []
    for root, _, files in os.walk(target_dir):
        for f in files:
            all_files.append(os.path.join(root, f))
            
    total_files = len(all_files)
    converted_count = 0
    
    for i, file_path in enumerate(all_files):
        if progress_callback:
            progress_callback(i + 1, total_files, f"ファイルを処理中: {os.path.basename(file_path)}")
            
        if is_image_file(file_path):
            try:
                # 画像のWebP変換処理
                with Image.open(file_path) as img:
                    # WebPに保存する準備
                    # EXIFなどのメタデータは透過webp保存時に維持可能
                    # output_path は拡張子を .webp に変更したもの
                    base, ext = os.path.splitext(file_path)
                    output_path = base + ".webp"
                    
                    # PillowでWebPとして保存
                    img.save(output_path, "WEBP", quality=quality)
                    
                # 変換元の画像ファイル（すでに.webpだった場合を除く）を削除
                if ext.lower() != ".webp":
                    os.remove(file_path)
                converted_count += 1
            except Exception as e:
                # 変換に失敗した画像は削除する（画像以外のテキストやリンクと同様に除外扱いとする）
                if os.path.exists(file_path):
                    os.remove(file_path)
        else:
            # 画像ファイル以外（テキスト、リンク、未対応のファイルなど）を削除
            try:
                os.remove(file_path)
            except Exception:
                pass # 削除失敗（シンボリックリンクなどの特殊ファイル）

    # 空になったディレクトリをクリーンアップ
    clean_empty_directories(target_dir)
    return converted_count

def create_zip_archive(source_dir, output_zip_path):
    """
    source_dir内のファイルをフォルダ構造を維持したままzip圧縮します。
    """
    # 出力先ディレクトリが存在することを確認
    os.makedirs(os.path.dirname(output_zip_path), exist_ok=True)
    
    with zipfile.ZipFile(output_zip_path, 'w', zipfile.ZIP_DEFLATED) as zip_ref:
        for root, _, files in os.walk(source_dir):
            for file in files:
                file_path = os.path.join(root, file)
                # 一時ディレクトリからの相対パスでアーカイブ内に格納
                arcname = os.path.relpath(file_path, source_dir)
                zip_ref.write(file_path, arcname)

def process_single_archive(archive_path, quality=50, log_callback=None, progress_callback=None):
    """
    単一の圧縮ファイルを処理するメインフロー。
    """
    if log_callback is None:
        log_callback = lambda msg: None
    if progress_callback is None:
        progress_callback = lambda current, total, msg: None

    archive_name = os.path.basename(archive_path)
    log_callback(f"【処理開始】 {archive_name}")
    
    # 拡張子の確認
    ext = os.path.splitext(archive_path)[1].lower()
    if ext not in [".zip", ".rar", ".7z"]:
        raise ValueError(f"対応していない圧縮形式です: {ext}")
        
    # RARの場合は事前にツール確認
    rar_tool = None
    if ext == ".rar":
        log_callback("RAR解凍ツールを探索中...")
        rar_tool = find_rar_tool()
        if not rar_tool:
            raise RuntimeError("RARファイルの展開に必要な外部ツール（7-Zip または WinRAR）が見つかりません。インストールしてください。")
        log_callback(f"解凍ツールが見つかりました: {os.path.basename(rar_tool)}")
        
    # フォルダパスの設定
    parent_dir = os.path.dirname(os.path.abspath(archive_path))
    old_temp_dir = os.path.join(parent_dir, "old_temp")
    new_temp_dir = os.path.join(parent_dir, "new_temp")
    
    # 一時作業用ディレクトリの作成
    with tempfile.TemporaryDirectory() as temp_dir:
        log_callback("アーカイブを一時フォルダに解凍中...")
        extract_archive(archive_path, temp_dir, rar_tool)
        
        log_callback("画像をWebPに変換し、不要ファイルを削除中...")
        converted = convert_images_in_dir(temp_dir, quality, progress_callback)
        log_callback(f"画像変換完了: {converted} 枚の画像を処理しました。")
        
        # 新しい圧縮ファイル名（拡張子は .zip に統一）
        base_name = os.path.splitext(archive_name)[0]
        new_zip_name = f"{base_name}.zip"
        
        # 一時zipファイルをtemp_dirの外に作成する
        temp_zip_dir = tempfile.mkdtemp()
        temp_zip_path = os.path.join(temp_zip_dir, "_output_.zip")
        
        try:
            log_callback("WebP画像を再圧縮中...")
            create_zip_archive(temp_dir, temp_zip_path)
            
            # フォルダ移動と配置
            # 1. new_tempに新しいzipを配置
            final_zip_path = os.path.join(new_temp_dir, new_zip_name)
            os.makedirs(new_temp_dir, exist_ok=True)
            if os.path.exists(final_zip_path):
                os.remove(final_zip_path)
            shutil.move(temp_zip_path, final_zip_path)
            log_callback(f"新ファイルを配置しました: new_temp\\{new_zip_name}")
        finally:
            shutil.rmtree(temp_zip_dir, ignore_errors=True)
        
        # 2. old_tempに元のファイルを移動
        os.makedirs(old_temp_dir, exist_ok=True)
        final_old_path = os.path.join(old_temp_dir, archive_name)
        if os.path.exists(final_old_path):
            os.remove(final_old_path)
        shutil.move(archive_path, final_old_path)
        log_callback(f"旧ファイルを移動しました: old_temp\\{archive_name}")
        
    log_callback(f"【処理成功】 {archive_name}\n")
    return final_zip_path
